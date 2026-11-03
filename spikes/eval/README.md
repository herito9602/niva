# E1: LLM tool-choice eval

Measures whether a local Ollama model picks the right v1 tools for Niva (design.md D3/D4, spike E1).
Cases live in `eval/commands.yaml` (Spanish and English). The runner sends each utterance to
`/api/chat` with the v1 tool schemas and inspects only the model's first response. No tool is executed.

## Run

```
ollama pull qwen3.5:2b
uv run spikes/eval/run_e1.py --model qwen3.5:2b
uv run spikes/eval/run_e1.py --model qwen3.5:2b --model <other> --limit 10
uv run spikes/eval/run_e1.py --model qwen3.5:2b --cpu
```

Options: `--model` (repeatable), `--cases` (default `eval/commands.yaml`), `--limit N`, `--cpu` (`num_gpu 0`).
Each model is warmed once, then all cases run with `think=False`, `temperature=0`.
Per-model JSON goes to `spikes/eval/results/<model>-<timestamp>.json` (git-ignored).

## Metrics

| Metric | Meaning |
|---|---|
| Correct tool % | Cases where every expected call is present with matching key args (any order) and no other call was made. `none` cases pass only with zero calls. |
| Reply language % | Of cases where the model returned text, the share whose text matches the request language. Heuristic: `3 x (accented letters, ñ, ¿, ¡)` + Spanish stopword count vs English stopword count. Ties or no signal are `undecidable` and excluded (reported separately). Replies that come with tool calls are templated in production (D4), so this mostly measures chit-chat and `none` cases. |
| Unrequested risky | Number of `close_app` / `power_action` calls that did not match an expected call. Because nothing is executed, this stands in for "risky actions without confirmation". The SafetyGate protects production regardless; report both. |
| Latency | Wall time of the chat call (warm model): mean, p50, p95 over all cases, and p95 over action-path cases (no `web_search`, chit-chat or follow-up cases). |

## Gates (design.md, specs)

- Correct tool >= 90%
- Reply language >= 95%
- 0 unrequested risky calls
- LLM tool-choice mean latency <= 800 ms (action path)
- End to end (measured in task 3.5, not by this runner): mean < 2.5 s and p95 <= 3.0 s

The original gate "p95 tool-choice latency <= 700 ms" is retired: measured LLM p95 is ~1.1-1.2 s (see design.md "Slice 0 results and decisions"). `run_e1.py` still prints its old p95 <= 700 ms line (`GATE_P95_MS`) for information.

Case notes: `reminder_followup` cases (reminder without a time) expect `create_reminder` called WITHOUT a time (the
model must not invent one); the core asks the follow-up question from a template. Changed 2026-11-02 because
the cases originally expected `none`, which contradicted the design; applied to every model. Reminder times are checked as 24-hour `HH:MM`.

## Provenance (reproducibility)

- Ollama 0.40.13.
- `qwen3.5:4b-q4_K_M`, digest `d8b0f5e9760cd1682034f292d7ef72ec46f432149be0df7574bf2d6e92e38c04` (4.2B, Q4_K_M). The tag is mutable; compare the digest before comparing results.
- Run settings (`run_e1.py`): `think=False`, `temperature=0`, `keep_alive="30m"`, one warm-up call. `num_ctx` is not set by `run_e1.py` (Ollama default for that run); the 4096 quoted in `spikes/voice/S1_RESULTS.md` co-residency is the S1 setup.
- STT used alongside (S1): faster-whisper `small` from `Systran/faster-whisper-small`, snapshot `536b0662742c02347bc0e980a01041f333bce120`, model.bin SHA-256 `3E305921506D8872816023E4C273E75D2419FB89B24DA97B4FE7BCE14170D671`.
