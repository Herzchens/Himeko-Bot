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

class TtsRequest(BaseModel):
    text: str
    voice: str
    speed: float = 1.0

def get_voice(voice_name: str):
    if tts is None:
        return voice_name
    # If the voice parameter is a path to an existing audio file, clone/encode it
    if os.path.isfile(voice_name):
        if voice_name not in voice_cache:
            print(f"Encoding custom voice clone from file: {voice_name}")
            try:
                voice_cache[voice_name] = tts.encode_reference(voice_name)
            except Exception as e:
                print(f"Failed to encode reference voice {voice_name}: {e}")
                return voice_name
        return voice_cache[voice_name]
    return voice_name

@app.post("/v1/tts")
async def tts_endpoint(req: TtsRequest):
    if tts is None:
        raise HTTPException(status_code=500, detail="VieNeu-TTS engine is not initialized.")
    try:
        resolved_voice = get_voice(req.voice)
        audio = tts.synthesize(req.text, voice=resolved_voice, speed=req.speed)
        
        out_buf = io.BytesIO()
        sf.write(out_buf, audio, samplerate=24000, format="WAV")
        wav_bytes = out_buf.getvalue()
        
        return Response(content=wav_bytes, media_type="audio/wav")
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=7799)
