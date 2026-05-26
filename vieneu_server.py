import os
import io
import sys

# Ensure dependencies can be imported
try:
    from fastapi import FastAPI, HTTPException
    from fastapi.responses import Response
    from pydantic import BaseModel
    import uvicorn
    import soundfile as sf
except ImportError as e:
    print(f"Missing dependency: {e}. Please run: pip install fastapi uvicorn soundfile")
    sys.exit(1)

app = FastAPI()
tts = None
voice_cache = {}

try:
    from vieneu import Vieneu
    print("Initializing VieNeu-TTS engine (this may download models on first run)...")
    tts = Vieneu()
    print("📢 VieNeu-TTS initialized successfully.")
    try:
        voices = tts.list_preset_voices()
        print("Available preset voices:")
        for desc, v_id in voices:
            print(f"  • {v_id}: {desc}")
    except Exception as e:
        print(f"Could not list preset voices: {e}")
except Exception as e:
    print(f"❌ Failed to load VieNeu-TTS: {e}")

import numpy as np

class TtsRequest(BaseModel):
    text: str
    voice: str
    speed: float = 1.0

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
        audio = tts.infer(req.text, voice=voice_data)
        audio = change_speed(audio, req.speed)
        
        sample_rate = getattr(tts, "sample_rate", 24000)
        out_buf = io.BytesIO()
        sf.write(out_buf, audio, samplerate=sample_rate, format="WAV")
        wav_bytes = out_buf.getvalue()
        
        return Response(content=wav_bytes, media_type="audio/wav")
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7799)
