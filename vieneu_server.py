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
        kwargs["codec_device"] = args.device
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
voice_cache = {}

class TtsRequest(BaseModel):
    text: str
    voice: str
    speed: float = 1.0
    temperature: float = 0.3  # Lower temperature for stable intonation

def load_custom_voice(voice_path: str):
    if voice_path not in voice_cache:
        base, _ = os.path.splitext(voice_path)
        txt_path = base + ".txt"
        if not os.path.isfile(txt_path):
            raise ValueError(f"Reference text file not found: {txt_path}")
        with open(txt_path, "r", encoding="utf-8") as f:
            ref_text = f.read().strip()
        ref_codes = tts.encode_reference(voice_path)
        voice_cache[voice_path] = {"codes": ref_codes, "text": ref_text}
    return voice_cache[voice_path]

def get_voice_data(voice_name: str):
    if tts is None:
        raise ValueError("VieNeu-TTS engine is not initialized.")
    if os.path.isfile(voice_name):
        return load_custom_voice(voice_name)
        
    v_name_lower = voice_name.lower().strip()
    try:
        presets = tts.list_preset_voices()
        for item in presets:
            desc, v_id = item if isinstance(item, tuple) else (item, item)
            v_id_lower = v_id.lower()
            desc_lower = desc.lower()
            if v_id_lower == v_name_lower or v_id_lower in v_name_lower or v_name_lower in v_id_lower or v_name_lower in desc_lower:
                return tts.get_preset_voice(v_id)
                
        # Fallback to the first available preset voice
        first_id = presets[0][1] if isinstance(presets[0], tuple) else presets[0]
        print(f"Warning: Voice '{voice_name}' not found, falling back to first preset: '{first_id}'")
        return tts.get_preset_voice(first_id)
    except Exception as e:
        print(f"Failed to match preset voice: {e}")
        
    try:
        return tts.get_preset_voice(voice_name)
    except Exception as e:
        print(f"Preset voice {voice_name} not found, using default: {e}")
        return tts.get_preset_voice(None)

def change_speed(audio, speed: float):
    if speed == 1.0 or speed <= 0:
        return audio
    old_indices = np.arange(len(audio))
    new_indices = np.arange(0, len(audio), speed)
    return np.interp(new_indices, old_indices, audio).astype(np.float32)

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
        print(f"⏱️ TTS synthesis timing: text='{req.text}', voice_match={t_voice:.3f}s, infer={t_infer:.3f}s, speed={t_speed:.3f}s, wav_write={t_wav:.3f}s, total={t_total:.3f}s", flush=True)
        
        return Response(content=wav_bytes, media_type="audio/wav")
    except Exception as e:
        print(f"❌ Error during synthesis: {e}", flush=True)
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=args.port)
