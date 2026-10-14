# Dictation AI

Hold a hotkey, speak, let go, and clean, ready-to-use text appears wherever your cursor is. A small macOS menu bar app, inspired by [Wispr Flow](https://wisprflow.ai).

## How it works

1. Put your cursor in any text field in any app.
2. **Hold** the hotkey (default **Option+Space**) and speak.
3. **Release** it. Your speech is transcribed, tidied up, and pasted at the cursor.

```
 hold hotkey ─► record mic ─► transcribe ─► clean up ─► paste at cursor
                                  │             │
                    Cloud: OpenRouter Whisper   OpenRouter language model
                    Local: whisper.cpp          (default: Claude Haiku 5.5)
```

- **Transcription** runs either in the cloud (OpenAI Whisper through [OpenRouter](https://openrouter.ai), the default) or locally on your Mac with [whisper.cpp](https://github.com/ggml-org/whisper.cpp) using one of three bundled models. You pick in Settings; there is no automatic fallback between them.
- **Clean-up** sends the transcript to a language model that removes filler words, applies self-corrections ("Tuesday, no, Wednesday"), and adds punctuation, without changing what you meant.
- **Paste** puts the result on the clipboard, sends Cmd+V, and restores whatever was on your clipboard before.

If clean-up fails, you still get the raw transcript. Temporary OpenRouter failures are retried automatically.

## Features

- Menu bar app: no Dock icon, no main window, with a status icon for idle, listening, processing and error
- Configurable hotkey, microphone, transcription engine (cloud or local model) and clean-up model
- A Usage window: audio seconds, words and cost for the last 24 hours and all time, split by transcription and clean-up
- Size-capped rolling log files (5 MB total) that never contain what you said

## Prerequisites

| Requirement | Notes |
|---|---|
| A Mac with Apple Silicon, macOS 15 or later | Developed and tested on an M1 Pro. The local models use the Apple GPU (Metal). |
| Xcode Command Line Tools | `xcode-select --install` |
| Rust | via [rustup](https://rustup.rs) |
| Node.js and npm | a current version; developed with Node 24 |
| cmake | `brew install cmake`, needed to compile whisper.cpp |
| An [OpenRouter](https://openrouter.ai) API key with credit | Needed for clean-up and for cloud transcription. OpenRouter required at least $0.50 of balance for audio requests when this was tested. |
| About 15 GB of free disk space | 1.2 GB of models, plus build output: a development build is about 8.5 GB and a release build adds about 3.5 GB. |

## Setup

```bash
# 1. Get the code
git clone https://github.com/asadalikazim/dictation-ai.git
cd dictation-ai

# 2. Add your OpenRouter key (this file is gitignored)
cp .env.example .env
#    then edit .env and set OPENROUTER_API_KEY=...

# 3. Download the Whisper models (about 1.2 GB, checksum-verified, resumable)
./scripts/fetch-models.sh

# 4. Install frontend dependencies
npm install

# 5. Run in development
npm run tauri dev
```

The first build compiles whisper.cpp and takes a few minutes. The build fails with a clear message if `.env` or the model files are missing.

To build the app itself:

```bash
npm run tauri build -- --bundles app
# result: src-tauri/target/release/bundle/macos/dictation-ai.app (about 1.1 GB)
```

### Permissions

macOS will ask for two permissions, and the app needs both:

- **Microphone**, prompted on first use.
- **Accessibility**, to send the paste keystroke. Enable it under System Settings > Privacy & Security > Accessibility.

When running with `npm run tauri dev`, these attach to your terminal app rather than to Dictation AI. The built `.app` gets its own entries.

## Using it

Click the menu bar icon for:

- **Usage…** to see what you have processed and what it cost
- **Settings…** to change the hotkey, microphone, transcription engine and model, and clean-up model
- **Show Logs in Finder**
- **Quit**

## Where things live

| What | Where |
|---|---|
| Settings and usage tally | `~/Library/Application Support/com.asadkazim.dictationai/` (`settings.json`, `usage.json`) |
| Logs | `~/Library/Logs/com.asadkazim.dictationai/` |
| Whisper models in a built app | inside the app bundle, under `Contents/Resources/resources/models` |

The development run and the built app share the same data, because they share a bundle identifier.

## Things to know

- **The API key is compiled into the app.** Anyone with the built `.app` can extract it, so don't share the build. Use a dedicated OpenRouter key with a spending limit.
- **Audio and text leave your machine** in cloud mode, and text does in local mode (for clean-up). Review your OpenRouter privacy settings.
- **Local transcription works offline**, but clean-up then fails and the raw transcript is pasted, after about 15 seconds of retries.

## Troubleshooting

| Symptom | Fix |
|---|---|
| Build stops with "OPENROUTER_API_KEY is missing" | Create `.env` from `.env.example` and set the key. |
| Build stops with "Whisper model files are missing" | Run `./scripts/fetch-models.sh`. |
| Nothing is pasted | Check the Accessibility permission, and the menu bar icon (a red ring means an error). Look in the logs. |
| whisper.cpp fails to build after changing the macOS version setting | `cargo clean -p whisper-rs-sys --release` in `src-tauri`, then rebuild. CMake caches the target. |

## Documentation

- [PRODUCT_OVERVIEW.md](PRODUCT_OVERVIEW.md): what the app is, the experience, principles, scope and trade-offs
- [TECHNICAL_BREAKDOWN.md](TECHNICAL_BREAKDOWN.md): architecture, per-component decisions, permissions, packaging and measurements
