# Himeko Bot

> A personal Discord bot written in Rust for text-to-speech, AI chat, voice-session utilities, rank management, and small server tools.
>
> **Work in progress.** **#J4F project — partially made with AI.**

---

## Features

- **Multi-backend Text-to-Speech (TTS)**
  - **gTTS** — lightweight Google TTS backend
  - **MsEdge** — Microsoft Edge neural voices
  - **Supertonic** — local or remote Supertonic synthesis server
  - **OpenAI-compatible** — custom OpenAI-compatible TTS APIs such as F5-TTS or viXTTS
  - **VieNeu-TTS** — Vietnamese TTS with local CPU/GPU modes and supported presets such as `Ly` and `Binh`
- **Per-guild voice sessions** with isolated runtime state and guarded session ownership
- **Bounded TTS scheduling** with ordered playback and process-wide/per-guild concurrency limits
- **Automatic local TTS server lifecycle** for supported loopback Supertonic/VieNeu configurations
- **Voice auto-rejoin** with bounded retry/backoff after unexpected disconnects
- **Vietnamese/English auto-detection** for TTS voice selection
- **Emoji and text normalization** for cleaner spoken output
- **Per-user voice gender preference**
- **AI chat integration** through Gemini or Groq via `/ask` and direct bot mentions
- **Discord mention safety** for AI output and webhook logging
- **Multi-guild rank system** with nickname/rank reconciliation and persistence
- **Atomic hot reload** through `/reload` for reload-safe configuration
- **Console chat bridge and webhook logging**
- **Valorant custom matchmaker** with random team/map generation

---

## Commands

Himeko currently registers 14 global slash commands.

| Command | Description |
|---|---|
| `/ask <question>` | Ask the configured AI provider a question |
| `/ping` | Show Discord WebSocket and HTTP latency |
| `/join` | Join your current voice channel and start a TTS session |
| `/leave` | Leave the active voice session when you are allowed to control it |
| `/gender <gender>` | Change your preferred TTS voice gender |
| `/reload` | Atomically reload reload-safe settings from `config.yml` |
| `/echo <message>` | Send a message as the bot; owner-only |
| `/makecustom` | Create randomized Valorant teams and choose a map |
| `/up @user...` | Increase rank level for one or more users; Administrator required |
| `/down @user...` | Decrease rank level for one or more users; Administrator required |
| `/remove @user...` | Remove rank state and restore nickname state; Administrator required |
| `/rescan` | Reconcile the current guild's member nicknames with rank storage; Administrator required |
| `/leaderboard` | Display the current guild's rank leaderboard |
| `/autorename on\|off` | Toggle automatic rank nickname enforcement; Administrator required |

You can also mention Himeko directly, for example `@Himeko <message>`, to use the AI mention path when AI access is enabled for your account.

### Permissions

Himeko has three application-level user access levels:

- **Owner** — full bot-level access, including `/echo` and session preemption
- **Allowed user** — TTS, AI, join, and own-session controls
- **Unknown user** — no TTS/AI/session-control access

Rank administration additionally checks Discord **Administrator** permission. The bot itself needs suitable guild permissions, including **Manage Nicknames** and **Manage Roles**, for rank operations.

---

## Tech Stack

- **Language:** Rust
- **Discord:** Serenity + Poise
- **Voice:** Songbird
- **AI backends:** Google Gemini, Groq
- **Configuration:** `serde_yaml`
- **Async runtime:** Tokio
- **Logging:** `tracing`

---

## Getting Started

### Prerequisites

- Rust toolchain (`rustup`)
- `ffmpeg` available in `PATH`
- A Discord bot token and application ID
- An API key for Gemini or Groq if AI is enabled
- Provider-specific dependencies if using a local TTS backend

### Setup

```bash
# 1. Clone the repository
git clone https://github.com/Herzchens/Himeko-Bot.git
cd Himeko-Bot

# 2. Create your local configuration
cp config.example.yml config.yml
# Edit config.yml and replace the example values

# 3. Build and run
cargo run --release
```

`config.yml` and local runtime/database files should not be committed with production secrets.

See [`config.example.yml`](./config.example.yml) for the current configuration schema and examples.

---

## Configuration Notes

### TTS provider validation

Supported `tts.provider` values are:

```text
gtts
msedge
supertonic
openai
vieneu
```

Himeko validates active-provider settings at startup and during reload instead of silently accepting invalid provider options.

For **gTTS**, both top-level values must remain zero because gTTS does not support rate or pitch adjustment:

```yaml
tts:
  provider: "gtts"
  rate: 0
  pitch: 0
```

### Hot reload

`/reload` validates the new configuration before replacing runtime state. Some settings are intentionally **startup-only** and require a full process restart when changed:

- `bot.token`
- `bot.application_id`
- `console_chat`
- `logging.webhook_url`
- the complete `rank` configuration

Other reload-safe settings are applied atomically only after validation succeeds.

### Multi-guild rank configuration

The legacy single-guild rank fields are still supported, while `rank.guilds` can define independent rank configuration for multiple guilds. When the same guild is present in both forms, the explicit `rank.guilds` entry takes precedence.

Each enabled rank guild requires a non-zero target role, leaderboard channel, at least one rank name, and a positive `stars_per_rank` value.

---

## VieNeu-TTS Setup & Acceleration

Himeko's bundled `vieneu_server.py` is validated against **VieNeu 2.7.0**. Do not install an unpinned/latest VieNeu release into this environment because newer releases may use a different dependency or runtime contract.

### 1. Python 3.12 environment

Windows example:

```powershell
py -3.12 -m venv venv
.\venv\Scripts\python.exe -m pip install --upgrade pip
.\venv\Scripts\python.exe -m pip install -r requirements-vieneu.txt
```

### 2. Optional GPU dependencies

For the supported VieNeu 2.7.0 GPU extra:

```powershell
.\venv\Scripts\python.exe -m pip install -r requirements-vieneu.txt "vieneu[gpu]==2.7.0"
```

Actual CUDA/LMDeploy availability still depends on your NVIDIA driver, GPU, Python environment, and upstream VieNeu runtime.

### 3. Configuration

Example:

```yaml
tts:
  provider: "vieneu"
  rate: 0
  pitch: 0
  vieneu:
    - server_url: "http://127.0.0.1:7799"
      female: "Ly"
      male: "Binh"
      speed: 1.0
      autostart: true
      mode: "fast"
      device: "cuda"
      temperature: 0.3
      pitch: 0
```

Notes:

- Local autostart is intended for loopback server URLs; remote provider URLs are not remapped to localhost.
- Readiness is verified through the provider health endpoint before the engine is considered available.
- Shared local-process ownership is preserved across safe reload transitions.
- Non-zero VieNeu `pitch` is rejected by the current runtime contract.
- CPU `turbo` remains the most portable mode; accelerated modes depend on the local VieNeu installation and hardware.

### 4. Realtime TTS tuning

| Setting | Suggested value | Notes |
|---|---|---|
| `max_chars` | `120–180` | Keeps individual synthesis requests reasonably small |
| `mode` | `fast` on a supported GPU | Use CPU `turbo` when GPU acceleration is unavailable |
| `temperature` | `0.3` | Stable default for supported VieNeu presets |

Runtime scheduling is bounded to avoid unbounded synthesis fan-out: up to **3 concurrent synthesis jobs per guild** and **12 process-wide**, while preserving admission order for playback.

---

## Production Deployment

For a small Linux server, Himeko can be deployed as a release binary rather than keeping the full source tree in the runtime directory.

Build natively on the target Linux environment or in a compatible Linux CI/build environment:

```bash
cargo build --release
```

Optional production size reduction:

```bash
strip target/release/himeko-bot
```

A minimal `systemd` service can use the directory containing `himeko-bot`, `config.yml`, and `database.yml` as its working directory:

```ini
[Unit]
Description=Himeko Discord Bot
After=network.target

[Service]
Type=simple
WorkingDirectory=/path/to/himeko-bot-app
ExecStart=/path/to/himeko-bot-app/himeko-bot
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

Keep a known-good binary backup before replacing a production executable.

---

## Validation

The repository includes a hardening CI workflow covering:

- `cargo fmt --check`
- `cargo check --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- debug tests
- release tests
- diff hygiene

The hardened runtime baseline has also been validated with native Windows builds, native Ubuntu release builds, and live Discord runtime checks including multi-guild voice sessions.

### Dependency security

`cargo audit` may report advisories originating from upstream/transitive dependencies. These should be investigated rather than suppressed solely to make the audit output green.

---

## Project Status

Himeko is an actively developed personal project. Configuration and provider contracts may change between releases; use `config.example.yml` from the same commit as the binary you deploy.

---

## License

[GPL-3.0](./LICENSE)
