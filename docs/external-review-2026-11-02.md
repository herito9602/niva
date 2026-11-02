# External review of `niva-mvp` — handoff to the implementing agent

**From:** an independent review session (different agent, no shared context).
**Date:** 2026-11-02.
**Audience:** the agent implementing `openspec/changes/niva-mvp`.
**Status:** advisory, NOT authoritative. Nothing here overrides `openspec/`. Every item marked
`[PENDING-OWNER]` needs the human's decision before you act on it.

## How to use this document

- Each finding cites `path:line` so you can verify it yourself. Please do: I read the artifacts,
  I did not run anything (my shell was blocked by a sandbox ACL error, so I have no empirical
  results and make no claims about runtime behavior).
- Findings are ordered by *what would be expensive to get wrong*, not by severity labels.
- Section 9 lists things that are correct and should NOT be changed. Read it before refactoring,
  so you don't "fix" a deliberate decision.
- Where I recommend a scope cut, it is a proposal for the owner, not an instruction.

## What I reviewed

`openspec/config.yaml`, `openspec/changes/niva-mvp/{proposal,design,tasks}.md`, all 11 files under
`openspec/changes/niva-mvp/specs/*/spec.md`, `docs/BACKLOG.md`, and the three
`spikes/*/RESULTS.md` plus `spikes/island` and `spikes/voice/spike_voice.py`.

Verified states: there is no `.git` yet (`.git/HEAD` does not exist), and there is no
`openspec/specs/` (nothing archived). So the repo is pre-slice-1, exactly as `tasks.md:1.1` says.

---

## 1. BLOCKING: the tool-loop spec contradicts design decision D4

This is the only finding I would call blocking, because of the archive rule in
`openspec/config.yaml:53` ("Merge delta specs into openspec/specs before archiving"). If you archive
as written, you ship a requirement the code will not satisfy.

- `specs/assistant-core/spec.md:17-28` — "Tool-calling loop": *"The core MUST let the model call
  tools, feed back each tool's real result, and produce a final reply"* and the "Loop ends" scenario.
- `design.md:16` (D4) — the opposite: *"Single-valued reply source… tool results are never fed back
  to the LLM except for the `web_search` summary call"*, plus `[design.md:16]` "single response only".
- `tasks.md:93` (2.9) — already acknowledges it: *"remove or demote the '6 LLM steps' cap
  (single-response-only) and fix the design text"*.
- `proposal.md:53` — "Approach" still justifies the loop with the spike finding about `"(dry run)"`
  results causing loops. That rationale is moot if results are never fed back. Stale text.

**Action:** rewrite the `assistant-core` requirement to describe the actual design (one LLM
response → validate → run → template reply; the only second call is the no-tools `web_search`
summary). Fix the `proposal.md:53` sentence and the `assistant-core` "Assumptions" line about
max iterations. Do this **before** writing `assistant.rs`, otherwise the test you write from the
spec will encode the wrong contract.

Note the dependency: this also settles carry-over item CO2 (`design.md:256`, `web_search` mixed with
a risky call). Pick the precedence now and write it into both `assistant-core` and `web-search`
specs; today neither spec defines the mixed case.

---

## 2. The risk-retirement order is inverted — biggest expected-value change

Unproven things, in order of how much they can kill the project:

1. **E1** (`design.md:183`) — is there a local model that hits ≥90% correct tool, ≥95% reply
   language, p95 ≤ 700 ms? The spike's own verdict is that the baseline does not qualify:
   *"A 2B local model is fast enough but not reliable enough for the product"*
   (`spikes/pc-control/RESULTS.md:33`).
2. **S1** (`design.md:181`) — does Whisper + the LLM both fit in 6 GB VRAM while meeting p95 ≤ 400 ms?
   `design.md:234` lists this as an open question. The latency budget (`design.md:51`) sums to
   **1.9 s against a 2.0 s gate** — zero margin, built from targets, with a spike baseline of 3.75 s.
3. **S4** (`design.md:186`) — can "hey niva" be trained at all, with recall ≥ 90% at a usable
   threshold? This is the product's headline feature, and it sits in slice 5 (`tasks.md:113`).

Meanwhile slice 1 (`tasks.md:40-81`) is island + mascot + the full reducer: ~1800-2400 lines
(`tasks.md:9`) that validate **none** of the three above.

**Recommendation [PENDING-OWNER]:** insert a "slice 0" before 1.A:

- `eval/commands.yaml` (~40 cases) + `bin/niva-eval.rs`, run E1 against qwen3.5:2b and 2-3 larger
  candidates. Reuse the spike's dry-run harness; this is cheap.
- Run S1 on the same GPU with the chosen model loaded.
- Timebox a feasibility probe for S4 (can openWakeWord's training pipeline produce anything usable
  for "hey niva"?). Do not ship it; just learn whether the headline is reachable.

If E1 and S1 pass, proceed as planned with confidence. If they fail, the architecture needs
rethinking (cloud adapter, BYOK, or a non-voice-first product) and you will have learned it in days
instead of after 8k lines.

`tasks.md:16` already says "Decision needed before apply: Yes". This is that decision.

---

## 3. Strategic: the local-first tax is being paid without the full privacy benefit

`proposal.md:5` sells *"fast, private"*. But D7 (`design.md:19`) uses **edge-tts**, so every spoken
reply sends its text to a Microsoft endpoint, over an unofficial, fragile API. STT and the LLM are
local; TTS is not. The design documents the trade-off honestly — that part is good — but the product
position is now inconsistent with the architecture's cost.

The cost of local-first is large: 4 toolchains (Rust/TS/Python/SQLite), a PyInstaller sidecar that
antivirus will flag (`docs/BACKLOG.md:12`), CUDA redistribution
(`design.md:233`), VRAM contention, model-load ordering (`design.md:63`), OOM downgrade
(`design.md:68`).

**Recommendation [PENDING-OWNER]:** force one position, and write it into `proposal.md`:

- **A. Truly local:** Piper as the default TTS, edge-tts as an opt-in setting. Keeps the privacy
  claim honest. Accepts worse prosody.
- **B. Cloud-first / BYOK:** cloud STT + TTS + a user-supplied key (the Hitodama model already noted
  in `docs/BACKLOG.md:46`). Kills the Python sidecar, the CUDA distribution and the 6 GB VRAM
  requirement, and incidentally fixes "friends without a GPU". Loses the privacy claim.

`design.md:19` already allows "a setting to force Piper", so option A is a small delta.

---

## 4. `web-search` is the weakest value-per-risk feature in v1

- The spec's own example is *"who won the last Champions League?"*
  (`specs/web-search/spec.md:12-15`).
- D5 (`design.md:17`) proposes Wikipedia REST or DuckDuckGo Instant Answer. Neither answers that
  question: DDG IA is empty for most queries; Wikipedia returns the competition article. S3's pass
  rule (`design.md:184`) is "usable text for ≥ 7 of 10", which is a low bar that still misses the
  motivating example.
- It is the only path needing a **second** LLM call, so it is > 2 s by construction
  (`design.md:52`), plus prompt-injection hardening (`design.md:149`).

**Recommendation [PENDING-OWNER]:** cut `web_search` from v1. It buys little, costs a free-but-
fragile dependency, a second LLM round trip, an injection surface, and one of the 11 capabilities.
If it must stay, change the spec's example to something a free source actually answers, so the eval
does not encode an unreachable criterion.

---

## 5. Architecture: the reducer is losing its purity, and 8 states are more than v1 needs

**Purity leak.** `design.md:67` and the matrix row `design.md:115` declare `SidecarReady` /
`SidecarDown` as *"service-level flags, handled in any state (not only the states in the matrix)"*.
That is exactly the property that makes `fn reduce(state, event)` valuable: every cell defined,
every cell testable (`tasks.md:59` promises a test iterating all 17 rows × 8 states). Flags outside
the matrix mean the coverage test cannot be total.

**Suggestion:** a small supervisor owns `voice_available` and feeds the reducer a first-class
`VoiceReadinessChanged{ready}` input. Then `voice_available` is reducer state, the matrix stays
total, and the coverage test in task 1.13 stays honest.

**State count.** Proposal's product rules (`proposal.md:31`) name 8 mascot states; `design.md:78-88`
maps them 1:1 to assistant states. For a v1 whose only flows are "command → action → confirm →
done", `success` and `error` could collapse into one `done{outcome}` state, and `reminder-alert`
could be a view overlay rather than a state. That removes a third of the matrix and keeps all 8
mascot poses (the pose table is a UI concern, `tasks.md:75`). The 1:1 mapping is a nice invariant but
it is not required by any spec — check `specs/mascot/spec.md:22` before changing it.

**Sizing.** `tasks.md:9-19` already flags the 400-line budget as High risk for *every* slice. My
read: honor that. Slice 1 should land as 1a/1b/1c/1d with `size:exception` on 1a (mostly copied
spike code), exactly as proposed. Resist merging them.

---

## 6. The operability surface is missing entirely

Nothing in the artifacts covers what happens on a friend's PC *before* the island appears:

- First run: mic permission, model download, GPU presence check, CUDA availability.
- Settings: force Piper, pick model, rebind shortcut, disable autostart.
- Diagnostics: a "doctor" that says *which* of the degraded states applies, in one screen.
  `design.md:61` has three excellent Ollama-degraded states, but they are only surfaced in the
  tray tooltip and debug window.
- Uninstall, and auto-update (or an explicit "no updates in v1" statement).

The design handles runtime degradation with real care (`design.md:59-68`) and handles *installation*
not at all. For "owner plus a few friends" that gap is where the support burden will live:
unsigned SmartScreen warning, no Ollama, PyInstaller tripping Defender.

**Recommendation:** add a `first-run` capability (or at least tasks in slice 5) covering a doctor
command, and note the install-time failure modes in `docs/BACKLOG.md` so they are not forgotten.

---

## 7. Traceability sweep — cheap, do it before archiving

Spec "Assumptions to confirm" sections were written pre-design and are now stale. Each of these is
answered somewhere in `design.md`:

| File | Stale assumption | Answered by |
|---|---|---|
| `specs/action-safety/spec.md:60` | "timeout duration is not stated" | `design.md:243` (10 s) |
| `specs/action-safety/spec.md:62` | multi-substring behavior unspecified | `design.md:146` (ask which) |
| `specs/island-shell/spec.md:43` | "idle period before auto-hide not defined" | `design.md:85` (3 s settle) |
| `specs/speech-io/spec.md:47-49` | metric definition, p95, GPU scope | `design.md:49-57` |
| `specs/voice-activation/spec.md:48` | "listening timeout unspecified" | `design.md:81` (6 s / 15 s cap) |
| `specs/assistant-core/spec.md:65` | "maximum tool-loop iterations is a design item" | moot under D4; see §1 |
| `docs/BACKLOG.md:19` | reasons about "STT and LLM in the cloud" | superseded by D1/D2 |
| `spikes/voice/RESULTS.md:15` | "STT moves to a cloud streaming API" | reversed by D1 (GPU-only v1) |

The last two are the confusing ones: a reader who starts at the spike will conclude the opposite of
D1. Since `design.md:216` says `spikes/` stays untouched, add a one-line pointer in
`docs/BACKLOG.md` rather than editing the spike.

---

## 8. Smaller items, in order

1. **Naming debt.** "Niva" vs Lada Niva / Nivea is already flagged (`docs/BACKLOG.md:7`). It is
   cheap now and expensive later: `productName`, bundle identifier, window/tray titles, wake phrase,
   and the trained model name all carry it. `tasks.md:46` renames "Niva Island" → "Niva" in slice 1;
   consider whether to settle the final name in that same pass.
2. **Reminder scope.** Wall-clock + IANA tz + DST re-resolution (`design.md:173`) is heavy for a
   single-user desktop app that can read the local tz. Keep `due_at_utc` + local wall time; drop the
   DST test requirement from v1 unless the owner travels.
3. **Fullscreen.** The truth table (`design.md:156-167`) depends on `QUNS_*` values that are marked
   `[verify]`. A simpler v1: "foreground window on the island's monitor AND its rect covers the
   monitor rect AND it is not a shell/maximized window". Ship the table only if S5 shows it is
   needed. Keep the `AudioCue` fallback (`design.md:169`) — that is the valuable part.
4. **Latency caveat worth writing down.** The budget counts the 400 ms VAD hangover toward the 2 s
   mean (`design.md:51`). That is honest and correct, but it means 20% of the budget is spent before
   STT starts. If S1 comes in at 400 ms p95 there is *no* slack left anywhere. Consider stating the
   target as "≤ 2.0 s mean with zero slack" so nobody plans against a comfortable margin that does
   not exist.
5. **`config.yaml:12`** still has `strict_tdd: false`. `tasks.md:37` says reducer work is test-first
   from 1.6. Flip the flag in the same PR that lands the test runner (task 1.5), so the config stops
   disagreeing with the plan.

---

## 9. Do NOT "fix" these — they are the strongest parts

Listing them so a later refactor does not undo them:

- **`Approved<T>` with crate-private constructors** (`design.md:22`, `design.md:145`). This is the
  single best idea in the design: it makes an unauthorized risky action a *compile error*, not a
  runtime check. Do not weaken it into a boolean or an enum anyone can construct.
- **Substring-only matching for destructive targets, never fuzzy** (`design.md:146`,
  `specs/action-safety/spec.md:51`). Justified by a real incident in the spike
  (`spikes/pc-control/RESULTS.md:32`).
- **Protected process list + out-of-session refusal + min query length 3** (`design.md:146`).
- **Power restart/shutdown click-only** (`design.md:129`, `design.md:248`) — ambient speech cannot
  shut the machine down.
- **D4's "the LLM is never on the error path"** (`design.md:16`) and templates owning the common
  replies. This kills language mixing and typos at the exact place they hurt most.
- **Prompt-injection containment** (`design.md:149`): summary call with no tools, results quoted as
  untrusted data, capped output.
- **`web_search` can never reach `ToolRegistry`** (`design.md:149`).
- **No LLM feedback on reject/timeout** (`design.md:129`) — prevents the model from arguing.
- **The stale-id guard** (`design.md:45`): every id-carrying sidecar event validated. Cheap and
  removes a whole class of race bugs.
- **Honesty markers.** Keep `[verify]` labels and the "targets until measured, none is a result"
  discipline (`design.md:47`). The project's credibility rests on them.

---

## 10. Questions only the owner can answer

1. Does "private" mean *no text ever leaves the machine*? If yes, edge-tts cannot be the default
   (§3) and `web_search` becomes contradictory by construction.
2. Is v1 a **personal tool** or a **product to sell**? The MVP scope ("owner plus a few friends",
   `proposal.md:7`) says personal; the "Before selling" backlog section says product. The answer
   changes §4, §6 and the whole slice-5 plan.
3. If S4 fails (custom "hey niva" unreachable), is the product still worth shipping with a stock
   wake word or shortcut-only? Deciding now prevents a bad outcome at the end of slice 5.
4. Is the 2 s target a *requirement* or an *aspiration*? If it is a requirement, §2 becomes urgent.

---

## Suggested immediate actions for the implementing agent

Do these in order, and stop for owner input at the `[PENDING-OWNER]` marks.

1. Fix §1 (spec/design contradiction) — a docs-only change, unblocks slice 2 design work.
2. Add `eval/commands.yaml` + `bin/niva-eval.rs` and run E1 (§2). Report metrics, do not tune the
   gates to the result.
3. Run S1 on the same GPU with the chosen model loaded; report p95 and VRAM.
4. Present §2, §3, §4 and §10 to the owner as a single decision request.
5. Only then start slice 1.A.
6. Do the §7 sweep in the same PR as any spec edit, so traceability never regresses.

If the owner overrides any recommendation here, record the override in `design.md` next to the
relevant decision, not in this file — this file is a snapshot, `design.md` is the living record.
