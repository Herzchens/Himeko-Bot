import os
import io
import sys
import argparse

parser = argparse.ArgumentParser()
parser.add_argument("--mode", default="turbo", help="VieNeu-TTS mode: standard | turbo | fast | remote | xpu")
parser.add_argument("--port", type=int, default=7799, help="Port to listen on")
parser.add_argument("--device", default="cpu", help="Device to run on: cpu | cuda")
args, _ = parser.parse_known_args()
# Clear sys.argv to prevent deep libraries (like lmdeploy/argparse) from parsing unexpected flags and crashing
sys.argv = [sys.argv[0]]

# CRITICAL: Import and initialize VieNeu-TTS (CUDA context) FIRST to avoid DLL conflicts with subsequent C++ imports
tts = None
try:
    from vieneu import Vieneu
    kwargs = {}
    if args.mode == "standard":
        kwargs["backbone_device"] = args.device
        kwargs["codec_device"] = "cpu"
    elif args.mode in ("fast", "gpu"):
        kwargs["backbone_device"] = args.device
        kwargs["codec_device"] = "cpu"  # Always keep codec on CPU to bypass massive Windows CUDA decoding lag
        kwargs["memory_util"] = 0.05    # Limit KV Cache to 5% VRAM — enough for short TTS, saves gigabytes
    elif args.mode == "remote":
        kwargs["codec_device"] = args.device
    else:
        kwargs["device"] = args.device
    tts = Vieneu(mode=args.mode, **kwargs)
    print("📢 VieNeu-TTS initialized successfully.")
    try:
        voices = tts.list_preset_voices()
        print("Available preset voices:")
        for item in voices:
            desc, v_id = item if isinstance(item, tuple) else (item, item)
            print(f"  • {v_id}: {desc}")
    except Exception as e:
        print(f"Could not list preset voices: {e}")
except Exception as e:
    print(f"❌ Failed to load VieNeu-TTS: {e}")

# Import supplementary web-server dependencies later
try:
    from fastapi import FastAPI, HTTPException
    from fastapi.responses import Response
    from pydantic import BaseModel
    import uvicorn
    import wave
    import numpy as np
except ImportError as e:
    print(f"Missing dependency: {e}. Please run: pip install fastapi uvicorn numpy")
    sys.exit(1)

app = FastAPI()
custom_voice_cache = {}
preset_voice_cache = {}
preset_name_map = {}

def init_preset_cache():
    if tts is None:
        return
    try:
        presets = tts.list_preset_voices()
        for item in presets:
            desc, v_id = item if isinstance(item, tuple) else (item, item)
            data = tts.get_preset_voice(v_id)
            preset_voice_cache[v_id.lower()] = data
            preset_name_map[v_id.lower()] = v_id
            preset_name_map[desc.lower()] = v_id
        print(f"📦 Cached {len(preset_voice_cache)} preset voices at startup", flush=True)
    except Exception as e:
        print(f"Warning: Failed to cache preset voices: {e}", flush=True)

init_preset_cache()

class TtsRequest(BaseModel):
    text: str
    voice: str
    speed: float = 1.0
    temperature: float = 0.3
    pitch: int = 0

def load_custom_voice(voice_path: str):
    if voice_path not in custom_voice_cache:
        base, _ = os.path.splitext(voice_path)
        txt_path = base + ".txt"
        if not os.path.isfile(txt_path):
            raise ValueError(f"Reference text file not found: {txt_path}")
        with open(txt_path, "r", encoding="utf-8") as f:
            ref_text = f.read().strip()
        ref_codes = tts.encode_reference(voice_path)
        custom_voice_cache[voice_path] = {"codes": ref_codes, "text": ref_text}
    return custom_voice_cache[voice_path]

def get_voice_data(voice_name: str):
    if tts is None:
        raise ValueError("VieNeu-TTS engine is not initialized.")
    if os.path.isfile(voice_name):
        return load_custom_voice(voice_name)

    key = voice_name.lower().strip()
    if key in preset_voice_cache:
        return preset_voice_cache[key]

    for alias, v_id in preset_name_map.items():
        if alias == key or key in alias or alias in key:
            return preset_voice_cache[v_id.lower()]

    if preset_voice_cache:
        first_key = next(iter(preset_voice_cache))
        print(f"Warning: Voice '{voice_name}' not found, using fallback: '{first_key}'", flush=True)
        return preset_voice_cache[first_key]

    return tts.get_preset_voice(voice_name)

def change_speed(audio, speed: float):
    if speed == 1.0 or speed <= 0:
        return audio
    old_indices = np.arange(len(audio))
    new_indices = np.arange(0, len(audio), speed)
    return np.interp(new_indices, old_indices, audio).astype(np.float32)

def apply_pitch(audio, pitch_pct: int):
    if pitch_pct == 0:
        return audio
    factor = 1.0 + pitch_pct / 100.0
    old_len = len(audio)
    new_len = max(1, int(old_len / factor))
    resampled = np.interp(
        np.linspace(0, old_len - 1, new_len),
        np.arange(old_len),
        audio,
    ).astype(np.float32)
    return resampled

@app.post("/v1/tts")
async def tts_endpoint(req: TtsRequest):
    import time
    t_start = time.time()
    if tts is None:
        raise HTTPException(status_code=500, detail="VieNeu-TTS engine is not initialized.")
    try:
        t0 = time.time()
        voice_data = get_voice_data(req.voice)
        t_voice = time.time() - t0
        
        # If temperature is 0, fall back to default randomness (1.0)
        temp = req.temperature if req.temperature > 1e-5 else 1.0
        
        t0 = time.time()
        audio = tts.infer(req.text, voice=voice_data, temperature=temp)
        t_infer = time.time() - t0
        
        t0 = time.time()
        audio = change_speed(audio, req.speed)
        t_speed = time.time() - t0
        
        t0 = time.time()
        audio = apply_pitch(audio, req.pitch)
        t_pitch = time.time() - t0
        
        t0 = time.time()
        sample_rate = getattr(tts, "sample_rate", 24000)
        out_buf = io.BytesIO()
        # Convert float32 array (-1.0 to 1.0) to int16 array (-32768 to 32767) for native WAV writing
        audio_int16 = (np.clip(audio, -1.0, 1.0) * 32767.0).astype(np.int16)
        with wave.open(out_buf, "wb") as wav_file:
            wav_file.setnchannels(1)  # Mono
            wav_file.setsampwidth(2)   # 16-bit (2 bytes)
            wav_file.setframerate(sample_rate)
            wav_file.writeframes(audio_int16.tobytes())
        wav_bytes = out_buf.getvalue()
        t_wav = time.time() - t0
        
        t_total = time.time() - t_start
        print(f"⏱️ TTS synthesis timing: text='{req.text}', voice_match={t_voice:.3f}s, infer={t_infer:.3f}s, speed={t_speed:.3f}s, pitch={t_pitch:.3f}s, wav_write={t_wav:.3f}s, total={t_total:.3f}s", flush=True)
        
        return Response(content=wav_bytes, media_type="audio/wav")
    except Exception as e:
        print(f"❌ Error during synthesis: {e}", flush=True)
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=args.port)
