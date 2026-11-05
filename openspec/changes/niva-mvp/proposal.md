# Proposal: Niva MVP

## Intent

Windows 11 has no fast, bilingual voice assistant that can control the PC where your voice and your commands never leave your PC (speech recognition and the LLM run locally; only reply text may leave the PC if a cloud TTS voice is used, see design D7). Niva is a Jarvis-style assistant: you say "Hey Niva" or press a shortcut, an animated island at the top of the screen reacts, and Niva runs the action. Three throwaway spikes (`spikes/*/RESULTS.md`) showed the pieces work. This change turns them into a product-grade MVP.

**Users (v1):** the owner plus a few friends. No accounts, payments or licensing. An unsigned installer is acceptable. (This replaces "any PC user" in `openspec/config.yaml`.)

## Scope

### In Scope
- Floating island (transparent, always on top, top-center), tray icon, Ctrl+Shift+Space shortcut.
- Original procedural robot mascot (Canvas 2D, spring animations) whose look follows the assistant state.
- Voice: local "Hey Niva" wake word, real VAD (Silero), STT, streaming TTS.
- Spanish and English: detect the language of each request, reply in that language, one voice per language.
- Actions: open/close apps, volume/mute, web search, local reminders.
- LLM behind a swappable `LlmProvider` port, with a local Ollama adapter.
- A safety gate in code and a fullscreen policy (rules below).
- An unsigned Windows installer.

### Out of Scope
- Everything in `docs/BACKLOG.md` under "Before selling" and "Roadmap after the MVP": calendar sync, email, notifications, WhatsApp, dev mode, signing, accounts.
- Claude/cloud LLM adapter (the port allows adding it later).
- Paid services of any kind.
- Reusing anything from Coucou/"Mochi" except the technique (see Mascot).

## Product Rules

- **Safety:** risky actions (close app, power/lock/sleep) always need user confirmation. App code enforces this, not the prompt. Safe actions never ask. Destructive app matching uses substring matching only, never fuzzy matching.
- **Fullscreen:** when a fullscreen app is active, Niva only interrupts on its own for important events (reminders). An explicit wake word or shortcut always shows the island.
- **Mascot:** an original character with no image assets. States: idle (breathing/blinking), listening, thinking, acting, success, error/confused, confirmation-needed, reminder-alert. Coucou's code is MIT, but its character, name and media are reserved, so we copy the technique only.

## Capabilities

### New Capabilities
- `island-shell`: window, tray, shortcut, show/hide lifecycle
- `mascot`: state-driven procedural robot animation
- `voice-activation`: wake word plus shortcut activation
- `speech-io`: VAD, STT, streaming TTS, per-language voices
- `language-handling`: detect the request language and reply in it
- `assistant-core`: LLM port, tool-calling loop, warm-up
- `action-safety`: risky/safe classification, confirmation gate
- `pc-actions`: apps, volume/mute
- `web-search`: answer from web results
- `reminders`: create, list and fire local reminders
- `fullscreen-policy`: rules for interrupting over fullscreen apps

### Modified Capabilities
None (no existing specs).

## Approach

Tauri 2 (Rust) + React + TypeScript + Vite, as proven in the island spike. Hexagonal core: ports for the LLM, STT, TTS, wake word and actions. Tool results are realistic and never placeholders (the spike showed "(dry run)" results misled the model); under design D4 they are not fed back to the LLM, so each request takes a single LLM response, validation, execution and a template reply.

## Open Decisions (settle in design)

1. **STT** with no paid services: GPU faster-whisper on the RTX 3060, or a free cloud tier. Must keep the action path under 2.5 s.
2. **Rust-only vs Python sidecar.** Affects packaging and antivirus flags.
3. **Product LLM:** qwen3.5:2b invents risky actions and mixes languages. Compare a bigger local model using an eval set.
4. **Web search source** (free API vs. scraping vs. opening the browser).
5. **Training the custom wake word** and its false-activation threshold.

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Latency stays > 2 s | High | GPU/streaming STT, streaming TTS, warm-up |
| LLM triggers unrequested actions | Med | Gate in code; eval set |
| Custom wake word is weak or fires falsely | Med | Test with real noise; keep the shortcut |
| Friends' PCs lack a GPU | Med | Keep the STT/LLM port swappable |
| Mascot looks derivative | Low (coral look approved by the owner on 2026-11-05, tasks.md 1.25) | Original design review, repeated on any redraw |

## Delivery Slicing (ordered)

1. **First slice:** git, test runner, island shell, and mascot state machine driven by a debug switcher (no AI, no voice).
2. Assistant core: Ollama port, tool loop, safety gate, app/volume actions via text input.
3. Voice: wake word (stock model), VAD, STT, TTS, language handling.
4. Reminders and fullscreen policy.
5. Web search, custom "Hey Niva" model, installer.

## Rollback Plan

This is a greenfield product: each slice is an isolated PR. Revert the slice's commits. Spikes stay untouched as the reference.

## Success Criteria

- [ ] End-of-speech to start of reply: < 2.5 s on average (spike: 3.75 s). ~500 ms of latency was traded for TTS voice quality (edge-tts); owner's call, 2026-11-02. End-to-end p95 goal: <= 3.0 s (design, Slice 0).
- [ ] 0 false activations in 10 min idle; at most 1 in 30 min of music, video and talk.
- [ ] 0 risky actions without confirmation across the eval set.
- [ ] Reply language matches the request in at least 95% of eval commands.
- [ ] Correct tool on at least 90% of eval commands (ES and EN).
- [ ] The mascot visibly shows all 8 states; the island over fullscreen follows the policy.

## Proposal question round

These need user review:
1. ~~Should a reminder interrupt fullscreen with sound, or only visually?~~ **Answered:** visually plus a soft chime; no spoken text over fullscreen apps.
2. ~~Can friends without a GPU accept slower replies, or is that a blocker?~~ **Answered:** acceptable for v1. Measured CPU-only on the owner's laptop: 11-27 s per command (see spikes/pc-control/RESULTS.md). v1 targets PCs with a GPU; CPU-only support goes to the backlog.
3. ~~Web search: should Niva read a short answer aloud, or open the browser?~~ **Answered:** speak a short summary. This makes the free search source (Open Decision 4) a real design item: results must be fetched as text, not just opened in a browser.
4. ~~If confirmation gets no answer (timeout), should the default be cancel?~~ **Answered:** yes, no answer cancels the action.
