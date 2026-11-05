# Niva backlog — deferred items

Things decided or discovered during planning and spikes that we postponed on purpose.
Grouped by when they matter. Started 2026-11-01.

## Before selling (product and legal)
- [ ] **Trademark check for "Niva"** in class 9 (software): national registry + WIPO Global Brand Database. Conflicts to review: Lada Niva (cars), similarity with Nivea.
- [ ] **Mascot look check.** The planned new robot is inspired by AI-generated concept images. Before selling, search for similar registered designs; an AI origin does not rule out a resemblance to an existing one.
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
- [ ] **Re-measure end-to-end latency** after those changes; target < 2.6 s (raised from 2 s after choosing edge-tts and from 2.5 s after the VAD measurement; spike measured 3.75 s avg).
- [ ] **Repeat the false-activation test with real noise** (music with lyrics, videos, conversation).
- [ ] **Barge-in by voice** ("Hey Niva" while Niva is speaking). Needs acoustic echo cancellation so Niva does not wake on its own voice; v1 only allows barge-in with the shortcut.
- [ ] **Voice confirmation for risky actions** (the PC control spike confirms by typing y/n).

## Island / desktop (from spikes/island/RESULTS.md)
- [ ] **Reduce WebView2 memory while hidden** (~170 MB private today), e.g. lower the memory target level.
- [x] **Cache the Start menu app index** at startup (loading it takes ~3.6 s). Done in PR 2d-a: the index is built in the background at start-up (about 4 s) and refreshed every 10 minutes and after a miss; a command before it is ready answers "still loading". It is not persisted across runs.
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
- [ ] **Reduce unrequested risky calls** (other "tired" or "done for today" phrasings -> sleep with qwen3.5:4b, 5 of 16 held-out; "me voy a dormir" itself is now a valid request): prompt wording, fewer power synonyms in the tool description, or few-shot negatives; re-run E1.
- [ ] **Validate STT with the owner's real microphone and noise** (S1 used synthetic edge-tts audio); watch negations like "no apagues".
- [x] **Measure VAD hangover** (done 2026-11-05: 500 ms wait, target raised to 2.6 s, see spikes/vad/RESULTS.md; STT overlapped with LLM generation is still open in slice 3) (400 ms target) and STT overlapped with LLM generation. **Scheduled before slice 3**: VAD is the only unmeasured line in the latency budget, and a 600 ms result would break the 2.5 s mean (and the 3.0 s p95).
- [ ] **E1 non-determinism**: results vary between runs at temperature 0; run E1 3x and report the spread before final gates.

## From PR 2a review (2026-11-03)
- [ ] **Protect the voice sidecar process by its final name** (sidecar packaging, tasks 3.3 and 5.3): `niva*` and `ollama*` stems are protected, but a sidecar that runs as plain `python.exe` is not. Pick a dedicated executable name (or a rule based on the parent process) and add it to the protected list in `crates/niva-core/src/safety/matcher.rs`.


## Owner decision on sleep (2026-11-04)
- [x] **Sleep stays voice-confirmable, with a 10 s countdown** (done on `feat/sleep-countdown`, `tasks.md` Batch 27: the 1 s grace value and the voice "cancel" of slice 3 are still the owner's to tune and to build) (own safety PR before PR 2d): after "yes", show and say the countdown; the OS call happens only at the end and only if nobody cancelled. Any key or mouse input cancels, detected with `GetLastInputInfo` (no global keyboard hook); the shortcut cancels too; voice "cancel" joins in slice 3. Restart and shutdown stay click-only; lock has no countdown. Replaces the click-only idea from the PR 2c review.
- [x] **Decide whether "me voy a dormir" is a valid request**: yes, by owner decision of 2026-11-04. E1 case `adv-es-me-voy-a-dormir` now expects `power_action sleep`, recorded in `eval/results/E1_RESULTS.md`; the real E1 run (`E1-relabel`, PR 2d-a) matched the hand re-score: 98.0% tool selection, 0 unrequested risky calls, only `vol-es-unmute` failing. Its action-path latency mean was 812 ms against the 800 ms gate (warm laptop); re-run E1 on a cooler GPU before the final gates.

- [ ] **A reminder alert must stay visible when audio is unavailable** (slice 4): `end_alert_if_finished` ends the alert when chime and speech are done, so with `Instant` media (no sidecar) it opens and closes at once and the island never shows. Confirmed by the owner in the debug window on 2026-11-04 with "Hold audio" off. Add a minimum display time.

## From the sleep countdown review (2026-11-04)
- [ ] **The countdown fault reply drops earlier results of the same plan** (`crates/niva-core/src/state_machine.rs`, `on_countdown_fault`; `crates/niva-core/src/assistant.rs`, `hold_for_countdown`): if a volume change ran before the sleep and the input cannot be read, the reply says "I did not suspend the PC" without saying the volume changed. The reducer only has the `SleepNotWatched` template; keeping the lines needs a `fault_text` in `ToolsOutcome::SleepCountdown` and in `State`, plus a test. Not a few-line fix, so left out of the review round.

## From PR 2c review (2026-11-04)
- [x] **Re-warm the model after a reducer timeout, a `keep_alive` expiry or an Ollama restart** (done in PR 2d-a: `adapters/ollama/keeper.rs` warms up again after any failed or 10 s+ call and every 10 minutes). Original note: nothing re-warms it today (`warm_up` runs once from `build_requests` in `app/src-tauri/src/lib.rs`; `keep_alive` is set in `app/src-tauri/src/adapters/ollama/wire.rs`), so the first request after 30 idle minutes is a cold load inside the 15 s budget.
- [x] **`pick_assistant` skips the warm-up backoff when Ollama is not ready at the first probe** (`app/src-tauri/src/dev.rs`): this contradicts `design.md:62` on the autostart race. Done in PR 2d-a: the keeper never gives up and `dev::adaptive_assistant` chooses per request.
- [ ] **A queued job does not get its remaining deadline** (`app/src-tauri/src/worker_pool.rs`, `app/src-tauri/src/requests.rs`): a job that waited in the queue still gets the full `CHAT_TIMEOUT` (`app/src-tauri/src/adapters/ollama.rs`) when it starts, instead of what is left of the reducer's 15 s.
- [ ] **A late success in the 15-16 s window leaves a stale plan** (`app/src-tauri/src/requests.rs`, `crates/niva-core/src/assistant.rs`): the reducer gives up at 15 s but the call may still succeed before `CHAT_TIMEOUT` fires at 16 s, and the plan it leaves is not cleared.
- [ ] **A worker panic waits for the full timer and only reaches stderr** (`app/src-tauri/src/worker_pool.rs`): the worker survives the panic, but the request gets no event until the reducer's Llm timer fires, and the user sees nothing about it.

- [x] **The memory of cancelled requests can evict an id** (done in PR 2d-a: a monotonic `cancelled_upto` watermark replaces the list, with the test "however many others are cancelled after it") (`assistant.rs:124`, `:683`, `:892`, capped at 32): a worker stuck for R while 32+ other ids are refused lets a cancelled R store a held sleep. No sleep follows: the reducer drops events of an id that is not current, so no countdown starts, and the next `begin_user` clears it. Replace the bounded list with a monotonic watermark or a session generation, and test refusal after 32 other cancels.

## From PR 2d-a (2026-11-04)
- [x] **PR 2d-b: everything that needs the island to answer.** Done: the island's Confirm and Cancel buttons (part 1), the real `close_app` (WM_CLOSE, 3 s wait, UWP frames) and `power` adapters, enabled in release (part 2, batch 33). `SetSuspendState` is not used: it hung the owner's PC, sleep goes through the monitor-power broadcast on Modern Standby PCs. The debug Accept path stays and stays dry-run (`dev.rs` tripwire).
- [ ] **Sleep on non-Modern-Standby PCs is not implemented** (needs a real S3 machine to test). `win_power::apply` makes no call there and the reply says sleep is not available on this PC yet. Do not add a path (`SetSuspendState` or other) without testing it on such a machine.
- [x] **Localized app names.** Windows names in the system language: on an English Windows "bloc de notas" finds nothing (the typo match only rescues close spellings such as "calculadora" -> "Calculator"). Decide whether a small es/en alias table is worth it, or whether the prompt should ask the model for the Start menu name. Done in batch 29: `win_apps/aliases.rs` (es/en groups of built-in Windows apps, whole-query match).
- [ ] **Say why the model is not answering.** In release "Ollama not installed", "not running" and "model not pulled" all reply "I can't think right now. Is the language model running?"; only the log (stderr) says which. Needs a port for the health (or a richer `LlmError`) and the doctor screen above.
- [x] **`running_processes` cannot report a failed snapshot.** Done in 2d-b (batch 33): the port returns a `Result`; a failed snapshot is an error reply, never "not running".
- [ ] **Hand tests of the release exe are pending** (volume and app launch on the owner's real PC; see `tasks.md` Batch 28). The write path of `win_volume` and `ShellExecuteExW` were never run against real hardware by tests (two `#[ignore]`d tests exist; the read-only one was run).
- [ ] **Tray tooltip / doctor for the keeper.** The keeper logs to stderr only; there is no UI for "Ollama is down, waiting" yet.

## From the PR 2d-a review (2026-11-04)
- [ ] **The release has no log file.** The release exe is `windows_subsystem = "windows"` and the keeper, the app indexer and the adapters report with `eprintln!`, so in release every failure of the keeper ("Ollama not reachable", "model missing") and of the indexer ("PowerShell did not answer", "SystemRoot is not set", the backoff) is invisible. Needs a small rolling log file under the app data directory (and the tray tooltip / doctor above to read from it).
- [x] **Localized app names** (`bloc de notas`). Confirmed on this PC with the read-only index test (`the_real_start_menu_has_apps_with_names_and_ids`): "bloc de notas" -> nothing, because the Start menu says "Notepad". See also the earlier "Localized app names" item; same decision (alias table or ask the model for the Start menu name). Done in batch 29: `win_apps/aliases.rs` (es/en groups of built-in Windows apps, whole-query match).
- [ ] **`vol-es-unmute`: "Sacá el silencio" MUTES the PC in release.** E1 shows the model answers `set_volume mute=true` for it in 3/3 runs (the only failing case). With the debug dry-run PC it was only a wrong test; with the real Core Audio adapter it reverses the user's request. Safe action, but a user-facing risk: fix in the prompt/tool description (a PR that re-runs E1) or add a guard in the host, and add the case to the hand tests.
- [ ] **A tool call that outlives the 5 s tool timer still acts.** The ports are synchronous and an in-flight call cannot be cancelled; the reducer reports the timeout and drops the result, but the adapter call (open an app, change the volume) still completes. A user who retries after "that took too long" can apply the action twice (a relative volume change would apply twice). Needs either idempotent actions (absolute levels only) or a cancellable adapter.
- [ ] **Remove `Msg::NotAvailableYet`** (`crates/niva-core/src/replies.rs`, `replies/tests.rs`): the host no longer uses it since the release assembly (`RiskyNotAvailableYet` replaced it); remove it and its two translations in a cleanup.
- [ ] **Apps behind a shortcut with arguments cannot be opened.** `Get-StartApps` names shortcuts that have no AUMID of their own `Microsoft.AutoGenerated.{GUID}`; they may carry arguments or point at a script, so the allow-list drops them (on this PC: Task Manager, Event Viewer, Task Scheduler, Git Bash, Git CMD, PowerShell 7, "VLC media player - reset preferences"). Resolving the shortcut target (and checking it is an `.exe` without arguments) would bring back the useful ones.
- [ ] **Merge `PcActions::launch_app` into `open_app`.** `open_app` should return the launched entry's display name itself; `launch_app` (default: the requested name) sits beside it only because the test doubles in `crates/niva-core/src/safety/tests.rs` implement `open_app` and that module was out of scope in PR 2d-a.

## From task 5.1, web search (2026-11-05)
- [ ] **Friends' key setup before the installer (task 5.3).** Search works only with each user's own free Tavily key in `NIVA_TAVILY_KEY` (user environment variable); the key must never be shipped in the app (Tavily's terms forbid sharing it). Write the short "setup note" the replies point to (create a free key, `setx NIVA_TAVILY_KEY <key>`, restart Niva), and decide whether the installer or a settings screen should take the key instead of a manual `setx`. The replies say "See the setup note" before that note exists.
- [ ] **Hand tests of web search with a real key** (never run by tests): a question in Spanish and one in English; the next-eclipse question (the summary must not name a past eclipse); no key set (reply "needs a key", everything else works); a wrong key (rejected reply); the registry fallback (`setx` then start Niva from a program that was already running, e.g. a terminal opened before the `setx`); `cargo test -p niva real_tavily -- --ignored` once.
- [ ] **A search that outlives the tool timer.** The 4 s call timeout sits inside the 5 s tool timer, but other safe calls of the same response share that timer. Fine for one search; revisit if a plan can carry slow safe calls next to a search.

## From batch 29 (2026-11-04)
- [ ] **`navegador` / `browser` should open the default browser.** There is no Start menu entry for the default browser, so no alias group can reach it; it needs its own path (the default-browser association), still launching an indexed or system-resolved target, never model text.
- [x] **`close_app` needs the same aliases on process names (2d-b).** Done in batch 33: `niva-core/src/close_names.rs`, an exact table applied before the safety gate; no fuzzy matching, the gate and the protected list are unchanged.
- [ ] **Slice 3: preload the model on the wake word or the shortcut.** The model now leaves the GPU after 30 idle minutes, so the first request costs a 10 to 13 s cold load. Start loading when the wake word or the shortcut fires, so the load runs while the user speaks. Also measure the cold load with Whisper resident in VRAM (S1 only measured both already loaded).
- [ ] **A cold load over the 15 s budget fails the first request.** The `Timeout` re-warm finishes the load, so the retry works; decide whether the reply should say the model is waking up.

- [ ] **The acting hold eats into the reading time.** The island holds a quick result up to 900 ms behind `acting` (`viewStore.ts`), but the reducer starts the reading timer when the result arrives, so the message is on screen up to 900 ms less than `reading.rs` computes. Add that margin in the core if a short reply feels rushed.

## Ideas from OpenJarvis (github.com/open-jarvis/OpenJarvis, reviewed 2026-11-04)
Read through fetched summaries of selected files, not a full audit. Its safety and voice loop are weaker than ours; these are the parts worth taking.
- [ ] **Slice 3: clean text before speaking.** Strip markdown, URLs, emoji and code from a reply before TTS, so the voice does not read symbols (their `toSpeakableText`, added in PR #1145). Matters for the web_search summary and chat replies; template replies are already plain.
- [ ] **Slice 3: probe the Whisper compute type at start-up.** Ask CTranslate2 which compute types the device supports and fall back (`float16`, then `int8`/`float32`) instead of crashing the sidecar (`speech/faster_whisper.py`).
- [ ] **Slice 3 installer: skip the Python App Execution Alias stubs.** On Windows `python`/`python3` under `WindowsApps` can be 0-byte stubs that print "not found"; validate a candidate before trusting it (their fix in PR #1056). Only applies if the sidecar is not packaged with its own runtime.
- [ ] **Slice 5: a `doctor` screen.** One place reporting Ollama reachable, model pulled, sidecar healthy, microphone present, plus a short privacy report (what can leave the machine: `web_search` queries and the edge-tts text, nothing else, and nothing bound beyond loopback), reporting presence, never secret values.
- [ ] **Local per-turn trace.** Latency of STT, LLM and TTS plus the validation outcome, in a local file with NO prompt or reply text. Lets us see real-use latency instead of only E1.

## Real close_app and power (PR 2d-b part 2, 2026-11-05)
- [ ] **Hand-test the release build** (see the list in the report of batch 33): close a desktop app, close Calculator (UWP) by its Spanish name, close an app that shows a save dialog, close an app with no window, lock, restart and shutdown with an unsaved document open, and sleep with the 10 s countdown.
- [x] **A tray app is reported as closed when only its window went.** Done: the process is checked too within the 3 s budget; a desktop process that stays gives "I closed the window of X, but it keeps running in the background".
- [ ] **Closing a real UWP app is only hand-tested.** The AUMID match was read live on open frames (Sticky Notes, Settings) but no test posts a close; confirm Calculator open, minimized and suspended, Settings and Photos in the hand test.
- [ ] **Restart and shutdown replies are said before Windows may object.** `ExitWindowsEx` returns once the request is accepted; with unsaved work Windows shows its own "apps are preventing" screen and the reply "Restarting the PC." has already been given.

## Island confirm buttons (PR 2d-b part 1, 2026-11-05)
- [ ] **Hand-test the buttons on a real desktop** (capture toggle on an unfocusable window, margins, a first click without focus, 125% and 150% scaling, a second monitor).
- [ ] **Keyboard access to the confirmation** (the island is unfocusable by design): Enter and Esc would need a global key, which conflicts with the "no key hook" rule. Voice and the buttons are the ways for now.
- [ ] **Buttons while the island springs.** The reported rectangle is the settled one; during the 900 ms intro the drawn button can be a few pixels off (covered by the 6 px margin and the 500 ms guard). If it ever feels wrong, report per frame or delay the reveal of the buttons.
