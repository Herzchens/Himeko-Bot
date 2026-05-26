import os
import io
import sys
import argparse

# Ensure dependencies can be imported
try:
    from fastapi import FastAPI, HTTPException
    from fastapi.responses import Response
    from pydantic import BaseModel
    import uvicorn
    import soundfile as sf
    import numpy as np
except ImportError as e:
    print(f"Missing dependency: {e}. Please run: pip install fastapi uvicorn soundfile numpy")
    sys.exit(1)

parser = argparse.ArgumentParser()
parser.add_argument("--mode", default="turbo", help="VieNeu-TTS mode: standard | turbo | fast | remote | xpu")
parser.add_argument("--port", type=int, default=7799, help="Port to listen on")
parser.add_argument("--device", default="cpu", help="Device to run on: cpu | cuda")
args, _ = parser.parse_known_args()

app = FastAPI()
tts = None
voice_cache = {}

try:
    from vieneu import Vieneu
    print(f"Initializing VieNeu-TTS engine in mode: {args.mode} on device: {args.device}...")
    tts = Vieneu(mode=args.mode, device=args.device)
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
        # First pass: try exact or case-insensitive or description match
        for item in presets:
            desc, v_id = item if isinstance(item, tuple) else (item, item)
            v_id_lower = v_id.lower()
            desc_lower = desc.lower()
            if v_id_lower == v_name_lower or v_id_lower in v_name_lower or v_name_lower in v_id_lower or v_name_lower in desc_lower:
                return tts.get_preset_voice(v_id)
                
        # Second pass: gender fallback mapping (e.g. Binh -> male voice, Ly -> female voice)
        is_male_req = any(kw in v_name_lower for kw in ["nam", "binh", "tuyen", "vinh", "son", "male", "guy"])
        for item in presets:
            desc, v_id = item if isinstance(item, tuple) else (item, item)
            desc_lower = desc.lower()
            v_id_lower = v_id.lower()
            if is_male_req and ("nam" in desc_lower or "nam" in v_id_lower):
                return tts.get_preset_voice(v_id)
            if not is_male_req and ("nữ" in desc_lower or "nu" in desc_lower or "nữ" in v_id_lower or "nu" in v_id_lower):
                return tts.get_preset_voice(v_id)
                
        # Fallback to the first available preset voice
        first_id = presets[0][1] if isinstance(presets[0], tuple) else presets[0]
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
    if tts is None:
        raise HTTPException(status_code=500, detail="VieNeu-TTS engine is not initialized.")
    try:
        voice_data = get_voice_data(req.voice)
        
        # If temperature is 0, fall back to default randomness (1.0)
        temp = req.temperature if req.temperature > 1e-5 else 1.0
        audio = tts.infer(req.text, voice=voice_data, temperature=temp)
        audio = change_speed(audio, req.speed)
        
        sample_rate = getattr(tts, "sample_rate", 24000)
        out_buf = io.BytesIO()
        sf.write(out_buf, audio, samplerate=sample_rate, format="WAV")
        wav_bytes = out_buf.getvalue()
        
        return Response(content=wav_bytes, media_type="audio/wav")
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=args.port)
