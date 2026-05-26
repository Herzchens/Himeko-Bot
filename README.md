# Himeko Bot

> A personal Discord bot written in Rust — reads your messages aloud and answers your questions.  
> **Work in progress.** **#J4F Project, don't take it seriously, Partically made by AI**

---

## Features

- **Advanced Text-to-Speech (TTS) Engine Dispatcher** — Supports multiple backends:
  - **gTTS** — Standard Google TTS engine
  - **MsEdge** — Microsoft Edge high-quality neural voices
  - **Supertonic** — Local/remote Supertonic fast-synthesis engine
  - **OpenAI-Compatible** — Supports custom OpenAI-compatible TTS APIs (like F5-TTS or viXTTS)
  - **VieNeu-TTS** — Local state-of-the-art Vietnamese TTS engine with high-quality presets (`Ly`, `Binh`) supporting CPU, CUDA (GPU), and LMDeploy acceleration
- **Automatic Server Control** — Automatically spawns and monitors background `supertonic` or `vieneu_server.py` subprocesses on configured ports if they are not already running
- **Comprehensive Emoji Filtering** — Intelligently parses Discord emojis:
  - **Unicode Emojis** (`😂`, `👍`, etc.) are filtered out dynamically to prevent TTS engines from babbling English Unicode names
  - **Custom Guild Emojis** (`<:pepe_L:1234...>` or `<a:pepe:1234...>`) are cleanly expanded into their descriptive text names (`pepe L`, `pepe`)
- **Per-User Voice Gender Selection** — Stores TTS gender preferences per `UserId` dynamically across guilds rather than simple global/guild-wide defaults
- **Robust Auto-Rejoin** — Automatically detects sudden voice channel disconnections and attempts a reconnect with exponential backoff (1s, 2s, 4s)
- **Bilingual Auto-Detection** — Automatically switches between Vietnamese and English voices without manual commands
- **AI Chat Integration** — Supports Gemini and Groq backend AI responses via slash commands or direct bot `@mention`
- **Rank System** — Automatically manages member nicknames and ranks based on tier/star level configurations
- **Hot-Reload Support** — Instantly reload settings from `config.yml` on the fly using `/reload`
- **Custom Valorant Matchmaker** — Random maps and player splitting for standard competitive matches
---

## Commands

| Command | Description |
|---|---|
| `/ask <question>` | Ask the AI a question directly |
| `/reload` | Reload `config.yml` without restarting the bot |
| `@Himeko <message>` | Mention the bot in chat to trigger an AI response |
|`/gender <gender>` | Change the gender of the TTS (gTTS doesn't have gender) |
| `/join` | Make bot joins the voice room |
| `/leave` | Make bot leaves the voice room |
| `/up @user1 @user2 ...` | Increase rank by 1 level (Admin) |
| `/down @user1 @user2 ...` | Decrease rank by 1 level (Admin) |
| `/remove @user1 @user2 ...` | Remove rank and restore original name (Admin) |
| `/leaderboard` | Display server rank leaderboard |
| `/autorename on\|off` | Toggle auto-rename guard (Admin) |

Bot needs **Manage Nicknames** and **Manage Roles** permissions for the rank system.


TTS is passive — just type in a watched text channel while Himeko is in your voice channel.

---

## Tech Stack

- **Language**: Rust
- **Discord library**: [Serenity](https://github.com/serenity-rs/serenity)
- **Voice**: [Songbird](https://github.com/serenity-rs/songbird)
- **AI backends**: Google Gemini, Groq
- **Config**: `serde_yaml`
- **Async runtime**: Tokio
- **Logging**: `tracing`

---

## Getting Started

### Prerequisites

- Rust toolchain (`rustup`)
- `ffmpeg` installed and available in `PATH`
- A Discord bot token
- An API key for at least one AI provider (Gemini or Groq)

### Setup

```bash
# 1. Clone the repo
git clone https://github.com/Herzchens/Himeko-Bot
cd Himeko-Bot

# 2. Copy and fill in the config
cp config.example.yml config.yml
# Edit config.yml with your tokens and preferred settings

# 3. Build and run
cargo run --release
```

See `config.example.yml` for the full configuration helper.

---

## VieNeu-TTS Setup & Acceleration

VieNeu-TTS is a state-of-the-art Vietnamese TTS engine integrated into Himeko Bot. It supports high-quality offline voices (`Ly`, `Binh`) and can be run with GPU (CUDA) acceleration on Windows:

### 1. Automatic Python 3.12 Virtual Environment Setup
Since Windows environments can have conflicts with global Python versions (e.g. 3.13 or 3.14), you should set up a local Python 3.12 environment using Scoop:
```powershell
# Set up a local Python 3.12 virtual environment and install dependencies
.\setup_venv.ps1
```

### 2. Enabling GPU (CUDA) Acceleration (Optional but Recommended)
For ultra-fast, high-quality synthesis, install the PyTorch + CUDA package inside the virtual environment:
```powershell
.\venv\Scripts\python.exe -m pip install torch==2.6.0 torchaudio==2.6.0 --index-url https://download.pytorch.org/whl/cu124
```

### 3. Running & Configuration
Set the TTS provider to `"vieneu"` in your `config.yml`:
```yaml
tts:
  provider: "vieneu"
  vieneu:
    - server_url: "http://127.0.0.1:7799"
      voice_female: "Ly"
      voice_male: "Binh"
      autostart: true  # Automatically starts vieneu_server.py in the background
      device: "cuda"   # "cpu" | "cuda"
      mode: "fast"     # "turbo" (CPU) | "fast" (LMDeploy GPU accelerated)
      temperature: 0.3 # 0.3 for stable intonation, 0.0 for natural randomness
```
When `autostart` is enabled, the bot automatically spawns and manages the VieNeu-TTS server in the background and pipes logs to `vieneu_server.log` and `vieneu_server_err.log`.

---

## Project Status

This bot is under active development. Things may break, change, or disappear between commits.

---

## License

[GPL-3.0](./LICENSE)
