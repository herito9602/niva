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
- p95 tool-choice latency <= 700 ms (action path; the budget slot is ~600 ms mean)

Case notes: `reminder_followup` cases (reminder without a time) expect no `create_reminder`; the
follow-up question comes from a template. Reminder times are checked as 24-hour `HH:MM`.
