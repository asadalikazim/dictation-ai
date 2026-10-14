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
- Configurable hotkey, microphone and transcription engine (cloud or a local model). The cloud transcription model and the clean-up model are picked from short lists, or set to any [OpenRouter](https://openrouter.ai/models) model ID.
- Your OpenRouter key is entered in Settings and stored in the macOS Keychain, with a **Check key** button
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
| An [OpenRouter](https://openrouter.ai) API key with credit | Needed for clean-up and for cloud transcription. You paste it into Settings after the first launch and it is stored in the macOS Keychain. OpenRouter required at least $0.50 of balance for audio requests when this was tested. |
| About 15 GB of free disk space | 1.2 GB of models, plus build output: a development build is about 8.5 GB and a release build adds about 3.5 GB. |

## Setup

```bash
# 1. Get the code
git clone https://github.com/asadalikazim/dictation-ai.git
cd dictation-ai

# 2. Download the Whisper models (about 1.2 GB, checksum-verified, resumable)
./scripts/fetch-models.sh

# 3. Install frontend dependencies
npm install

# 4. Run in development
npm run tauri dev
```

The first build compiles whisper.cpp and takes a few minutes. The build fails with a clear message if the model files are missing.

Then add your OpenRouter key: click the menu bar icon, choose **Settings…**, paste the key into **OpenRouter key**, and press **Save**. **Check key** confirms OpenRouter accepts it. Without a key, cloud transcription and clean-up cannot run; local transcription still works and pastes the raw transcript.

To build the app itself:

```bash
npm run tauri build -- --bundles app
# result: src-tauri/target/release/bundle/macos/dictation-ai.app (about 1.1 GB)
```

### Permissions

macOS will ask for two permissions, and the app needs both:

- **Microphone**, prompted on first use.
- **Accessibility**, to send the paste keystroke. Enable it under System Settings > Privacy & Security > Accessibility.

macOS may also ask whether the app can use the Keychain item that holds your key. Choose **Always Allow**. Development builds are re-signed on every rebuild, so you may be asked again after rebuilding.

When running with `npm run tauri dev`, these attach to your terminal app rather than to Dictation AI. The built `.app` gets its own entries.

## Using it

Click the menu bar icon for:

- **Usage…** to see what you have processed and what it cost
- **Settings…** to set your OpenRouter key and change the hotkey, microphone, transcription engine and model, and clean-up model
- **Show Logs in Finder**
- **Quit**

## Where things live

| What | Where |
|---|---|
| Settings and usage tally | `~/Library/Application Support/com.asadkazim.dictationai/` (`settings.json`, `usage.json`) |
| Your OpenRouter key | The macOS login Keychain: service `com.asadkazim.dictationai`, account `openrouter-api-key` (visible in Keychain Access). Never in a file. |
| Logs | `~/Library/Logs/com.asadkazim.dictationai/` |
| Whisper models in a built app | inside the app bundle, under `Contents/Resources/resources/models` |

The development run and the built app share the same data, because they share a bundle identifier.

## Things to know

- **Your key is in the Keychain, not in the app or the repo.** The built app contains no key. Consider a dedicated OpenRouter key with a spending limit.
- **Audio and text leave your machine** in cloud mode, and text does in local mode (for clean-up). Review your OpenRouter privacy settings.
- **Local transcription works offline**, but clean-up then fails and the raw transcript is pasted, after about 15 seconds of retries.

## Troubleshooting

| Symptom | Fix |
|---|---|
| The error icon appears, or the logs say "No OpenRouter API key is set" | Open Settings and add your key, then use **Check key**. |
| **Check key** says the key was rejected | The key is wrong, revoked, or from a different account. Create a new one on OpenRouter and use **Replace**. |
| Build stops with "Whisper model files are missing" | Run `./scripts/fetch-models.sh`. |
| Nothing is pasted | Check the Accessibility permission, and the menu bar icon (a red ring means an error). Look in the logs. |
| whisper.cpp fails to build after changing the macOS version setting | `cargo clean -p whisper-rs-sys --release` in `src-tauri`, then rebuild. CMake caches the target. |

## Documentation

- [PRODUCT_OVERVIEW.md](PRODUCT_OVERVIEW.md): what the app is, the experience, principles, scope and trade-offs
- [TECHNICAL_BREAKDOWN.md](TECHNICAL_BREAKDOWN.md): architecture, per-component decisions, permissions, packaging and measurements
