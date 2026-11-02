# PC control spike results (2026-11-01)

Setup: Ollama `qwen3.5:2b` (local, RTX 3060 Laptop 6 GB, `think=False`), dry run, 6 Spanish commands.
LLM behind a provider port (`OllamaProvider` / `ClaudeProvider`); Claude not tested (no API credits).

| Check | Result |
|---|---|
| Start menu index | 278 apps (incl. Store apps) in 3.6 s |
| Right first tool for the command | 6/6 |
| Multi-action command ("open Spotify and set volume to 20") | Correct |
| Unrequested extra actions | 1/6 in the final run (a `shutdown` after "open Spotify"), blocked by the confirmation gate |
| Warm latency, command -> reply | 0.6-1.5 s |
| Cold start (model load) | 10-13 s on the first request |
| Reply language | Sometimes English for a Spanish command |

### Live run by the user (real actions, 7 commands)
- 7/7 correct actions, no unrequested actions; both `close_app` calls confirmed by the user.
- Ambiguity recovery works: "Explorador de Windows" -> tool returned candidates -> model picked "Explorador de archivos".
- Latency ~0.8-1.4 s (excluding time spent answering confirmations).
- Reply quality is weak: language mixing and typos ("Volución", "abreé").
- Bug found and fixed: `Get-StartApps` output used the console OEM code page; now forced to UTF-8.

### CPU-only run (`--cpu`, i7-11370H, no GPU offload, 5 commands)
- Same correct tool choices, but 11.5-26.7 s per command (GPU: ~1 s).
- The first request of each command costs 10-21 s: the system prompt + tool schemas are re-processed every time.
- Follow-up requests inside a command take 1.4-3.7 s.
- Local LLM on CPU is not usable for a voice assistant as-is.

## Findings
- **The confirmation gate must live in code, not in the prompt.** The 2B model called `lock`, `sleep` and `shutdown` unprompted in earlier runs; prompt rules reduced it but did not remove it.
- **Tool results must look like real results.** Returning "(dry run) would ..." made the model think the action failed and repeat it in loops.
- **Fuzzy matching is dangerous for destructive tools.** `close_app("Spotify")` once matched `FortiTray.exe`; closing now uses substring matching only.
- A 2B local model is fast enough but not reliable enough for the product; it validates the loop, not the quality.

## Decisions
- Keep the provider port; the product model choice (cloud vs bigger local) is open.
- Risky tools always require confirmation from the app; safe tools never do.
