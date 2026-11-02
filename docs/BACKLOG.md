# Niva backlog — deferred items

Things decided or discovered during planning and spikes that we postponed on purpose.
Grouped by when they matter. Started 2026-11-01.

## Before selling (product and legal)
- [ ] **Trademark check for "Niva"** in class 9 (software): national registry + WIPO Global Brand Database. Conflicts to review: Lada Niva (cars), similarity with Nivea.
- [ ] **Domain availability** (e.g. niva.app, getniva.com).
- [ ] **Business model: who pays for the AI.** Options: our subscription covering API costs, or the user brings their own API key. Affects pricing and onboarding.
- [ ] **Code-sign the installer** (v1 for friends ships unsigned; Windows SmartScreen will warn).
- [ ] **Accounts, payments and licensing** (out of v1: personal use plus a few friends).
- [ ] **Distribution packaging.** If a Python sidecar survives, PyInstaller executables are large and often flagged by antivirus.

## Project setup
- [ ] **`git init` with a `.gitignore` first** (spikes' `node_modules` and `target` hold ~3.9k files).
- [ ] **Set up a test runner with the first product code** (cargo test / vitest / pytest), then turn on strict TDD in `openspec/config.yaml`.

## Architecture decisions still open
- [ ] **Rust-only vs Python sidecar.** With STT and LLM in the cloud, openWakeWord (ONNX) could run from Rust and Python may not be needed.
- [ ] **LLM as a swappable port** (hexagonal): `LlmProvider` interface with one adapter per provider (Ollama today, Claude or others later).
- [ ] **Window overlap policy.** The island covers the browser tab strip; decide whether it shrinks, moves, or auto-hides over fullscreen apps.

## Voice (from spikes/voice/RESULTS.md)
- [ ] **Train a custom "Hey Niva" wake word model** (openWakeWord training pipeline). The spike used the stock "hey_jarvis" model.
- [ ] **Cloud streaming STT** instead of local Whisper (local `small` took ~1.7 s on CPU).
- [ ] **Real VAD for endpointing** (Silero) instead of a volume threshold; it cut commands off mid-phrase.
- [ ] **Streaming TTS playback** from the first audio chunk.
- [ ] **Re-measure end-to-end latency** after those changes; target < 2 s (spike measured 3.75 s avg).
- [ ] **Repeat the false-activation test with real noise** (music with lyrics, videos, conversation).
- [ ] **Barge-in by voice** ("Hey Niva" while Niva is speaking). Needs acoustic echo cancellation so Niva does not wake on its own voice; v1 only allows barge-in with the shortcut.
- [ ] **Voice confirmation for risky actions** (the PC control spike confirms by typing y/n).

## Island / desktop (from spikes/island/RESULTS.md)
- [ ] **Reduce WebView2 memory while hidden** (~170 MB private today), e.g. lower the memory target level.
- [ ] **Cache the Start menu app index** at startup (loading it takes ~3.6 s).

## PC control (from spikes/pc-control/RESULTS.md)
- [ ] **Pick the product LLM.** `qwen3.5:2b` is fast but invents extra actions; compare a bigger local model and Claude (needs API credits) on the same commands.
- [ ] **Small eval set of voice commands** to measure wrong or extra tool calls per model.
- [ ] **Support PCs without a GPU.** Local LLM on CPU took 11-27 s per command. Options: cache the system prompt + tools prefix, a smaller tool schema, or a free cloud tier (check current limits and terms).
- [ ] **Model warm-up at startup** (first local request takes 10-13 s).
- [ ] **Force the reply language** to match the user (the model sometimes answers in English).

## Ideas from Hitodama (hitodama.in, reviewed 2026-11-02)
Closest competitor found: voice-first Windows companion with an island, sold as a one-time purchase.
- [ ] **Bring-your-own free API key (BYOK)** as the business/cost model: Hitodama runs on the user's own Gemini API key, kept inside Google's free tier. Would solve "who pays for the AI" and PCs without a GPU. Verify free-tier availability per country (not offered everywhere; EEA/UK/CH require billing) and Google's terms.
- [ ] **Routines**: "every morning at 9, open Unity and Spotify". Reuses the reminder scheduler.
- [ ] **Scheduled actions**: "search Amazon for gift wrap on Nov 10 at 11:00".
- [ ] **Dictation anywhere**: hold a shortcut, talk, clean text is typed at the cursor. Reuses STT.
- [ ] **Circle to ask**: select a screen region and ask about it. Needs a multimodal model.
- [ ] **To-do list** in the island.
- [ ] **Control centre**: scroll the island to show music, system stats and quick settings.
- [ ] **Customization slots**: mascot skin, personality, voice.

## Roadmap after the MVP
- [ ] **Calendar sync for reminders** (v1 keeps reminders local). Outlook via Microsoft Graph is simpler; Google Calendar in testing mode forces re-login every 7 days.
- [ ] **v1.1 email summaries: Outlook first** (Microsoft Graph).
- [ ] **Gmail later**: reading mail is a restricted scope that needs Google verification plus a paid yearly third-party security audit (CASA).
- [ ] **Windows notification listener**: likely needs package identity (MSIX) and user permission; needs its own spike. WhatsApp has no API to reply.
- [ ] **Sending WhatsApp messages**: there is no official API for personal accounts (the Business API is for companies); automating WhatsApp Desktop's UI is fragile and may break its terms. Needs research before promising it.
- [ ] **"Dev mode"**: monitor AI coding agents (Claude Code hooks), TwoCode-style.
