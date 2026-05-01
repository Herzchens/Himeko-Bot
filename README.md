# Himeko Bot

> A personal Discord bot written in Rust — reads your messages aloud and answers your questions.  
> **Work in progress.** **#J4F Project, don't take it seriously, Partically made by AI**

---

## Features

- **Text-to-Speech** — Himeko joins your voice channel and reads messages in real time
- **Bilingual auto-detection** — automatically switches between Vietnamese and English voices without any manual input
- **AI integration** — ask questions via slash command or `@mention`; supports Google Gemini and Groq as backends
- **Rank System** — manage member ranks with configurable tiers and star levels
- **Hot-reload config** — update settings on the fly with `/reload`, no restart needed
- **Fully config-driven** — voices, provider, language rules, and bot behavior are all controlled via `config.yml`
- **Make custom valorant matches** - Random map and split player into 2 sides.
- **And more will be developed, open and issue if you want to request features.**
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
| `/rank_remove @user1 @user2 ...` | Remove rank and restore original name (Admin) |
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

## Project Status

This bot is under active development. Things may break, change, or disappear between commits.

---

## License

[GPL-3.0](./LICENSE)
