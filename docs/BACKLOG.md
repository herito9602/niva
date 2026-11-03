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
- [ ] **First run** (external review §6): mic permission, model download, GPU presence and CUDA availability checks.
- [ ] **Doctor screen**: one screen saying which degraded state applies (Ollama not installed / not running / model not pulled, sidecar down, slow mode), not only the tray tooltip.
- [ ] **Settings**: force Piper, pick the model, rebind the shortcut, disable autostart.
- [ ] **Uninstall and update**: uninstaller behavior, and auto-update or an explicit "no updates in v1" statement.
- [ ] **Install-time failure modes** to document: unsigned SmartScreen warning, Ollama missing, PyInstaller sidecar flagged by Defender.

## Project setup
- [ ] **`git init` with a `.gitignore` first** (spikes' `node_modules` and `target` hold ~3.9k files).
- [ ] **Set up a test runner with the first product code** (cargo test / vitest / pytest), then turn on strict TDD in `openspec/config.yaml` (done 2026-11-02: `strict_tdd: true`).

## Architecture decisions still open
- [ ] **Rust-only vs Python sidecar.** With STT and LLM in the cloud, openWakeWord (ONNX) could run from Rust and Python may not be needed.
  - Superseded for v1 by `openspec/changes/niva-mvp/design.md` D1/D2: GPU-local faster-whisper STT and a Python sidecar (a cloud adapter only if spike S1 fails). The "STT and LLM in the cloud" premise above no longer applies to v1.
- [ ] **LLM as a swappable port** (hexagonal): `LlmProvider` interface with one adapter per provider (Ollama today, Claude or others later).
- [ ] **Window overlap policy.** The island covers the browser tab strip; decide whether it shrinks, moves, or auto-hides over fullscreen apps.

## Voice (from spikes/voice/RESULTS.md)
- [ ] **Train a custom "Hey Niva" wake word model** (openWakeWord training pipeline). The spike used the stock "hey_jarvis" model.
- [ ] **Cloud streaming STT** instead of local Whisper (local `small` took ~1.7 s on CPU).
  - Superseded for v1 by `design.md` D1 (local GPU Whisper; v1 targets GPU PCs). `spikes/voice/RESULTS.md`'s "STT moves to a cloud streaming API" decision is reversed by D1; cloud STT stays only as the S1 fallback.
- [ ] **Real VAD for endpointing** (Silero) instead of a volume threshold; it cut commands off mid-phrase.
- [ ] **Streaming TTS playback** from the first audio chunk.
- [ ] **Re-measure end-to-end latency** after those changes; target < 2.5 s (raised from 2 s after choosing edge-tts; spike measured 3.75 s avg).
- [ ] **Repeat the false-activation test with real noise** (music with lyrics, videos, conversation).
- [ ] **Barge-in by voice** ("Hey Niva" while Niva is speaking). Needs acoustic echo cancellation so Niva does not wake on its own voice; v1 only allows barge-in with the shortcut.
- [ ] **Voice confirmation for risky actions** (the PC control spike confirms by typing y/n).

## Island / desktop (from spikes/island/RESULTS.md)
- [ ] **Reduce WebView2 memory while hidden** (~170 MB private today), e.g. lower the memory target level.
- [ ] **Cache the Start menu app index** at startup (loading it takes ~3.6 s).
- [ ] **Thinking pose: owner not fully convinced by the one-finger crown scratch (2026-11-03); revisit the look later.**

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

## From slice 0 (E1/S1, 2026-11-02)
- [ ] **Reduce unrequested risky calls** ("me voy a dormir" -> sleep with qwen3.5:4b): prompt wording, fewer power synonyms in the tool description, or few-shot negatives; re-run E1.
- [ ] **Validate STT with the owner's real microphone and noise** (S1 used synthetic edge-tts audio); watch negations like "no apagues".
- [ ] **Measure VAD hangover** (400 ms target) and STT overlapped with LLM generation. **Scheduled before slice 3**: VAD is the only unmeasured line in the latency budget, and a 600 ms result would break the 2.5 s mean (and the 3.0 s p95).
- [ ] **E1 non-determinism**: results vary between runs at temperature 0; run E1 3x and report the spread before final gates.

## From PR 2a review (2026-11-03)
- [ ] **Protect the voice sidecar process by its final name** (sidecar packaging, tasks 3.3 and 5.3): `niva*` and `ollama*` stems are protected, but a sidecar that runs as plain `python.exe` is not. Pick a dedicated executable name (or a rule based on the parent process) and add it to the protected list in `crates/niva-core/src/safety/matcher.rs`.
