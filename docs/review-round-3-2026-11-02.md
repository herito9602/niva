# Round 3 review note — read before continuing slice 1.B

**From:** the same independent review session as `docs/external-review-2026-11-02.md` (round 2).
**Date:** 2026-11-02, after the slice 0 commits.
**Audience:** the agent implementing `openspec/changes/niva-mvp`.
**Status:** advisory. Nothing here blocks the reducer (1.6-1.13). All four items are documentation or
one policy decision; three can be done in a single docs commit.

**Read this next to `design.md` § "Slice 0 results and decisions" (line 276).** The measurements are
good. The issues below are all about what the docs now say *around* those numbers, not the numbers.

---

## 1. Decision needed: no end-to-end p95 criterion exists, and the data already exceeds 2.5 s at p95

The chain, all documented, each step reasonable on its own:

- Gate was `LLM p95 <= 700 ms` (`spikes/eval/README.md:34`, `specs/speech-io/spec.md:49`).
- Measurement: **LLM p95 1110-1232 ms**. Stated plainly at `spikes/voice/S1_RESULTS.md:56`:
  *"the solo LLM p95 (1.1-1.2 s) already misses the E1 700 ms action-path gate"*.
- Gate replaced with **mean <= 800 ms** (`design.md:283`). Measured mean 760-776 ms → passes by 24 ms.
- Requirement raised 2 s → 2.5 s to fit edge-tts at ~900 ms first audio (`proposal.md:87`).

Net effect: the product has a **mean** criterion and **per-stage p95** gates, but no **end-to-end
p95** criterion. Using the measured numbers:

```
400 (VAD, still unmeasured) + 203 (STT p95) + 1110 (LLM p95) + 100 (tool) + 900 (TTS) ≈ 2.71 s
```

Roughly 1 command in 20 lands near or past 2.5 s, and the mean-based criterion cannot see it.
That may be an acceptable v1 trade — but it should be a decision, not a by-product.

**Proposed action:** add one line to `design.md` § Slice 0 and to `specs/speech-io/spec.md` (near
line 31) choosing one of:

- **A.** an explicit end-to-end **p95 <= 3.0 s** goal, measured in task 3.5 with real audio; or
- **B.** an explicit statement that **only the mean is contractual** in v1 and the tail is accepted,
  with the reason (voice quality was chosen over the 500 ms).

Either is defensible. Silence is not, because slice 3 is where it becomes visible and expensive.

**Acceptance:** `grep -n "p95" specs/speech-io/spec.md design.md` returns a statement of the
end-to-end percentile policy, not just per-stage gates.

---

## 2. Three stale references to the retired 700 ms gate, plus one README that contradicts its own data

| File | Line | Problem |
|---|---|---|
| `specs/speech-io/spec.md` | 49 | still states the p95 gates as "LLM ≤ 700 ms (E1)" |
| `design.md` | 55 | the "E1 reconciliation" paragraph still reasons about the p95 700 gate |
| `design.md` | 47, 51 | "targets until S1, S2 and E1 measure them" — they have measured; table still shows STT ≤ 400 ms and LLM ~600 ms against measured 190 ms and 776 ms |
| `spikes/eval/README.md` | 34 | gate list still says `p95 tool-choice latency <= 700 ms` |

And a real contradiction, not just staleness:

- `spikes/eval/README.md:36-37` says `reminder_followup` cases *"expect no `create_reminder`"*.
- `eval/commands.yaml:21-25` documents that this was **changed on 2026-11-02** precisely because
  "the cases originally expected `none`, which contradicted the design".

So the README describes the pre-change behavior. Anyone re-running the eval will expect the opposite
of what the file asserts.

**Proposed action:** update the four lines; in the README, record the change and that it was applied
to **every** model (that detail is what keeps E1 from looking retro-fitted, and it is currently only
in `commands.yaml`).

---

## 3. E1 is not reproducible: record the model digests

`qwen3.5:4b-q4_K_M` is a mutable Ollama tag. If it is re-pushed, 96.1% stops reproducing and nobody
can tell why. Whisper's provenance **is** recorded (`S1_RESULTS.md:15`, `mobiuslabsgmbh` CT2
conversion) — same treatment for the LLM.

**Proposed action:** add to `spikes/eval/README.md` and `S1_RESULTS.md` § Setup the output of
`ollama show qwen3.5:4b-q4_K_M` (digest) and the SHA of the CT2 model file, plus the exact
`think=False`, `temperature=0`, `num_ctx=4096` settings if not already captured in the raw JSON.

---

## 4. Wording: "raised from 2 s" vs "not loosened"

`proposal.md:87` says the target was **raised** from 2 s to 2.5 s. `design.md:283` says
**"stated openly, not loosened"**. Both are honest, but together they read as defensive.

**Proposed action:** one sentence in both places, naming the trade:
*"~500 ms of latency and the 'private' claim were traded for TTS quality; owner's call, 2026-11-02."*
Naming the trade is stronger than denying it, and it is consistent with the rest of the document's
style of stating costs plainly.

---

## What I am not asking you to change

- The slice 0 numbers and model choices. `small` + `qwen3.5:4b` are well argued and the evidence is
  in the repo.
- The S1 caveat (`S1_RESULTS.md:6-8`) and the "not measured" list (`:70`). Those two sections are
  why the numbers are believable — keep the same standard for the items above.
- Anything in the reducer (1.6 onward). This note does not touch `niva-core`.

## The one measurement I would still schedule early

`VAD 400 ms` is now the **only remaining "target, unmeasured"** line in the latency budget
(`design.md:283`). It is also cheap: a microphone and the spike script, no product code. If VAD
lands at 600 ms, the **mean** criterion breaks too, not only the p95. Worth knowing before slice 3
commits ~1500-2000 lines to the sidecar.
