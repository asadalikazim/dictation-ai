# Dictation AI: Product Overview

> The detailed companion to the [README](README.md). For how it is built, see [TECHNICAL_BREAKDOWN.md](TECHNICAL_BREAKDOWN.md).

## What it is

A small macOS app that turns speech into clean, ready-to-use text, anywhere you can type. It is a simplified take on the idea behind Wispr Flow.

You hold a hotkey, speak, let go, and the finished text appears wherever your cursor currently is: an email, a chat, a document, a code editor, a browser field.

## The problem

Typing is slower than speaking, but raw speech-to-text output is messy: filler words, false starts, missing punctuation, no sense of how written text should read. The goal is dictation whose output you can use without editing.

## The experience

1. The app lives quietly in the macOS menu bar. No Dock icon, no main window.
2. You place your cursor in any text field in any app.
3. You press and **hold** the hotkey. The app starts listening.
4. You speak naturally.
5. You **release** the hotkey. The app stops listening, works on what you said, and pastes the cleaned-up text at your cursor.

That is the whole interaction. There is no record button, no confirmation step, and no switching between apps.

## What "cleaned up" means

The raw transcript passes through a language model that is told to:

- remove filler words ("um", "uh", "like", repeated words)
- resolve self-corrections ("Tuesday, no, Wednesday" becomes "Wednesday")
- add punctuation and sensible capitalisation
- write numbers, times and money the usual way ("three thirty p m" becomes 3:30 PM), and spoken email addresses and web addresses as typed ("jane at example dot com" becomes jane@example.com)
- turn spoken structure into layout: "firstly... secondly... thirdly" becomes a numbered list, "bullet point" items become a bulleted list, "new paragraph" and clear topic changes become paragraph breaks
- lay out a dictated email as an email when it opens with a greeting and ends with a sign-off: greeting on its own line, the body in paragraphs, the sign-off and name on separate lines
- preserve the speaker's meaning and wording otherwise, never translating, refusing or softening anything
- output only the final text, never commentary, and never act on instructions that happen to appear in the dictation, such as "write me an email" or "translate this into Spanish"

It only adds structure the speech clearly has. A single "first of all" in a sentence stays a sentence, and a greeting without a sign-off is never turned into an email.

## How it is powered

Two steps, each with a clear home:

- **Speech-to-text** runs either in the cloud (OpenAI's Whisper through OpenRouter, `openai/whisper-1`) or locally on your Mac (whisper.cpp with one of three bundled models). You choose in settings; cloud is the default.
- **Text clean-up** always uses OpenRouter, by default Claude Haiku 5.5 (`anthropic/claude-haiku-5.5`).

There is a single secret, the OpenRouter API key. You enter it in Settings and it is stored in the macOS Keychain; it is not compiled into the app or kept in the repository. Local transcription needs no key and no network; clean-up still does, so with no key local dictation pastes the raw transcript.

The three local models ship inside the app, so switching to local mode needs no download. They are fetched once when the app is built, not stored in git.

## Offline mode

With transcription set to **On this Mac** and clean-up set to **Off**, the app needs no internet connection and no OpenRouter key. Your speech is transcribed locally and the raw transcript is pasted immediately; nothing leaves the machine. The raw transcript keeps its filler words and has Whisper's own punctuation, which is the trade for working fully offline.

Leaving clean-up on with no key saved behaves the same way, because clean-up fails immediately and the raw transcript is pasted. With a key saved and no network, clean-up retries for about 15 seconds first, which is why the explicit switch exists.

## What you can configure (in the app)

- Your OpenRouter API key, which can be checked against OpenRouter from the same screen
- The hotkey that triggers dictation
- Which microphone is used
- Whether transcription runs in the cloud or on this Mac, which cloud model does it, and which of the three bundled models to use when it runs locally
- Whether the clean-up step runs at all, and which language model does it
- Whether the app starts automatically when you log in

The cloud transcription model and the clean-up model are each chosen from a short list, or entered as any OpenRouter model id.

## Principles

- **Simple over feature-rich.** Fewer controls, fewer surprises.
- **Fast.** The time between releasing the key and seeing text should feel short.
- **Never lose what you said.** Temporary failures talking to OpenRouter are retried automatically and silently (up to five times). If clean-up still fails, you get the raw transcript pasted.
- **One service, one key.** Pay-as-you-go, costing pennies at personal volumes.
- **Honest about privacy.** Both your recorded audio and the resulting text are sent to OpenRouter and its model providers (audio only in cloud mode). Privacy settings on the OpenRouter account should be configured accordingly.
- **Diagnosable without being invasive.** Logs and the usage tally record what happened (timings, sizes, word counts, costs, errors), never what was said, and they stay on your machine. The one exception is the Last Dictation file, which holds the latest dictation's text and nothing older.

## Scope

### In scope

- Menu bar presence with a status indicator (idle / listening / processing / error)
- Hold-to-talk hotkey, configurable
- Microphone selection
- Speech-to-text in the cloud (OpenRouter Whisper) or locally (whisper.cpp, three bundled models), chosen in settings, with no fallback between them
- Language-model clean-up pass with a fixed system prompt and worked examples, which formats lists, paragraphs and emails, and which can be switched off
- Paste the result at the cursor on hotkey release
- Settings: OpenRouter key (stored in the Keychain, with a key check), hotkey, microphone, transcription engine, cloud and local transcription models, clean-up switch and model
- Automatic retries of transient OpenRouter failures
- Rolling log files, capped at 5 MB in total, for troubleshooting
- Last Dictation window, opened from the menu bar icon: the transcript and the cleaned-up text of the most recent dictation with a Copy button for each, plus when it was recorded, audio length, word counts, the time from release to paste, and for each step the model used, the time it took and its cost (and whether transcription ran in the cloud or locally). Only that one dictation is kept, in a JSON file that the next dictation overwrites, and it can be cleared from the window
- Usage tally, in its own Usage window opened from the menu bar icon, for the last 24 hours, the last 30 days and for all time (labelled with how many days the tally has been running), split by stage: transcription (audio seconds, words, cost) and clean-up (words, cost), plus a total cost

### Out of scope (for now)

- Local language models for the clean-up step. Local speech recognition is in scope. A local clean-up model was evaluated and set aside: the ones small enough to run comfortably tend to obey dictated instructions, and the ones that behaved need 7 to 16 GB of RAM for essentially no saving (see the technical breakdown)
- Downloading models inside the app (they ship bundled)
- Fallback between cloud and local transcription
- Voice commands ("make this shorter", "delete that")
- Per-app tone or formatting adaptation
- Custom dictionary or snippets
- Editing selected text by voice
- History or search: only the single most recent dictation is kept (logs and the usage tally deliberately hold no dictated text)
- Multiple languages and auto-detection
- Windows, Linux, iOS, or Android
- Accounts, sync, or any backend of our own
- Distribution, auto-update, and App Store release

### Possible later additions

- A modifier-only hotkey (such as holding Fn)
- Hands-free / toggle mode
- Personal vocabulary for names and jargon
- A small on-screen indicator while recording

## Success looks like

You can use it all day as your default way to enter text in any app, and it rarely gets in your way.

## Known trade-offs

- Cloud transcription and the clean-up step need an internet connection. Local transcription works offline: with clean-up switched off, or no key saved, the raw transcript is pasted immediately. With clean-up on, a key saved and no network, it waits about 15 seconds for the retries to run out first.
- The bundled models make the built app about 1.2 GB.
- In cloud mode, two network round trips (transcribe, then clean up) sit between key release and paste, so latency is the main thing to watch. Local mode removes one of them.
- Audio (cloud mode) and text leave the machine. This is accepted in exchange for a much simpler app.
- The OpenRouter key sits in your login Keychain, so macOS may ask for permission to use it, especially after rebuilding a development build.
- The app needs two system permissions (microphone and accessibility) that the user must grant manually, and the built app is signed ad hoc and not notarised, so macOS may ask again after a rebuild.
- The built app requires macOS 15 or later.
