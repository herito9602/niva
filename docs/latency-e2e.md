# Latency end to end: the hand test (task 3.5)

Goal (spec `speech-io`, design "Metric logging"): over the action path, with the real
microphone, the mean from the end of the user's speech to the first sound of Niva's answer
(`speak_started`) is **< 2.6 s** and the p95 is **<= 3.0 s**. The estimate was ~2.85 s at p95
(VAD 532 + STT 203 + LLM 1110 + tool 100 + TTS 900 ms). This page is how to measure it.

This is a HAND test: it opens the microphone, plays sound, changes the real volume and opens
real apps. Nothing here is run by the test suite.

## What is logged

One JSON line per request, in debug and release builds, with numbers and fixed words only:
**never** what you said, what Niva answered, an app name or a prompt.

- File: `%LOCALAPPDATA%\Niva\latency.jsonl` (for example `C:\Users\<you>\AppData\Local\Niva\latency.jsonl`).
  Set `NIVA_LATENCY_LOG` to another path to override it (the folder is created).
- Bound: at about 1 MiB the file is renamed to `latency.jsonl.1` (replacing the older one) and a new
  file starts. A write failure is reported once on stderr and never touches a turn.
- Clock: all `t_*` fields are milliseconds on ONE monotonic clock of the host (zero = app start),
  with the sidecar's end of speech mapped onto it. `at` is the wall clock (epoch ms), for time ranges.

| field | meaning |
|---|---|
| `req` | id of the turn's first request |
| `path` | what the turn did: `action` (the model chose safe tools, they ran and Niva answered), `search`, `chat`, `confirm` (the first sound is a confirmation question), `followup` (a question about a missing argument), `sleep` (the sleep countdown's announcement), `error` (the model, a tool or the plan failed), `cancelled` (the turn ended before it produced anything: a cancel, or the next command). A command that worked is never `cancelled`: if one is, that is a bug in the trace |
| `channel` | `voice` or `text` |
| `lang`, `whisper` | reply language; Whisper model of the running sidecar (voice turns) |
| `validation` | what the first model answer became: `run_safe`, `confirm`, `invalid`, `reply`, `follow_up`, `llm_failed` |
| `t_end_of_speech`, `t_transcript`, `t_request`, `t_llm_done`, `t_tool_done`, `t_speak_sent`, `t_first_audio` | the stage timestamps; a stage that did not happen is absent, never zero |
| `stt_ms`, `llm_ms`, `tool_ms`, `tts_ms` | stage durations. `stt_ms` is end of speech to transcript, so it includes the endpointer's silence wait (~530 ms) as well as Whisper; `llm_ms` and `tool_ms` add up the two model calls of a search. A failed model call counts its time in `llm_ms` too |
| `other_ms` | what those stages leave out of the total (hand-offs, queues); the named stages and `other_ms` add up to `total_ms` |
| `total_ms` | `t_first_audio - t_end_of_speech`: the number the goal is about |

There is no `t_llm_first_token`: the Ollama adapter does not stream, so there is no first token to
time. A turn that ends without any sound (voice down, text mode, a failed speech) is written without
`t_first_audio`, `tts_ms`, `total_ms`. A confirmation question is labelled `confirm` so it never
enters the action-path mean.

## Before you start

1. Ollama is running with the product model: `ollama list` shows `qwen3.5:4b-q4_K_M`.
2. Close other GPU-heavy programs (the figures assume the RTX 3060 is free; Whisper `small` and the LLM share it).
3. Quiet room, speakers at a normal level. Nobody else talking. Say the wake word and the command
   at your normal speed and distance.
4. Search commands need `NIVA_TAVILY_KEY` set in the same PowerShell window (without it the answer
   says search is not set up and the turn is labelled `error`, not `search`).
5. The tests below leave a Calculator and a Notepad window open and the volume at about 30%.
   Close the windows by hand. Do NOT ask Niva to close apps or touch the power in this test.

## Start the release build

PowerShell, from the repository (build once, and again after any code change):

```powershell
cd D:\trabajo\ProyectoAsistente\app
npm run tauri build -- --no-bundle
```

Then, in the SAME window (the app inherits the variables):

```powershell
$env:NIVA_VOICE_DIR = "D:\trabajo\ProyectoAsistente\voice"
$env:NIVA_WAKE_MODEL_DIR = "$env:LOCALAPPDATA\uv\cache\environments-v2\spike-voice-32308f98e9d78df1\Lib\site-packages\openwakeword\resources\models"
$env:NIVA_PIPER_DIR = "D:\trabajo\ProyectoAsistente\spikes\voice\models"
$env:NIVA_LATENCY_LOG = "$env:LOCALAPPDATA\Niva\e2e-1.jsonl"   # a fresh file for this session
# $env:NIVA_TAVILY_KEY = "..."                                  # for the 3 search commands
# Keep the app's and the sidecar's stderr (see "Keep the stderr" below):
$logs = "$env:LOCALAPPDATA\Niva\e2e-1-logs"
New-Item -ItemType Directory -Force $logs | Out-Null
Start-Process D:\trabajo\ProyectoAsistente\target\release\niva.exe `
    -RedirectStandardError "$logs\stderr.log" -RedirectStandardOutput "$logs\stdout.log"
```

The release exe has no console, so there is no log window; the two redirected files are the
log. Wait for **"voice ready"**:

- Hover the Niva tray icon (it may be in the hidden-icons arrow). The tooltip says `Niva: voice starting`
  while it loads, and **`Niva: voice ready`** when the microphone delivers its first frame. Expect about
  20 s for the models, plus up to 30 s more if Ollama is still warming up the LLM.
- If the tooltip says voice is off or restarting, stop here: fix that first (see `voice/README.md`).
  Measuring in text mode is not this test.

## Keep the stderr (so a missing command can be explained)

The latency file has one line per request. A command that never became a request (the wake word was
not heard, nothing was said, the speech was empty) leaves no line there. Those drops are in the
stderr of the app, which also carries the voice sidecar's stderr (the host forwards it line by line).
`Start-Process -RedirectStandardError` above writes it to `stderr.log` (PowerShell needs a different
file for stdout, hence `stdout.log`, normally empty). Use a new folder per session: starting again
with the same names overwrites the files of the last one.

What is in it, and what is not: the sidecar logs a reason code and, for a wake, its score; the app
logs ids, the kind of each speech and its length in characters, and the state of the Ollama keeper.
**No transcript, no reply, no prompt and no app name is written** (speech-io, "Discard reasons").
Before you share a file, search it for a word you said; there should be none.

| line | meaning |
|---|---|
| `discard reason=wake_while_speaking` | you said the wake word while Niva was still speaking: ignored on purpose |
| `discard reason=wake_while_listening` | a second wake word during a listen |
| `discard reason=wake_near_miss score=0.4x` | something close to the wake word, under the threshold |
| `discard reason=wake_unclaimed` | the wake word was heard but the app opened no listen (it was thinking, acting or asking a confirmation) |
| `discard reason=no_speech`, `empty_transcript`, `hallucination_gate` | the listen ended with no usable words |
| `whisper small on cuda`, `running Whisper on the CPU` | where Whisper runs (the CPU would break the target) |
| `[niva] listen N (Command)` | the app opened listen N |
| `[niva] speak N (Final, 15 chars)` | the app sent a speech (kind and length only) |
| `edge-tts failed before any audio` | the network voice fell back to Piper: a slow first sound (a 10 s `tts_ms` looks like this) |

Count the commands you said against the `listen` lines and the lines of `latency.jsonl`: a command
that is missing from the file but present as a `discard` was never heard as a command; one with no
trace at all never got through the microphone pipeline.

## Warm up (not measured)

The first command pays for cold caches (Whisper kernels, the LLM, the TTS voice). Do one throwaway
command and wait until Niva has finished speaking and the island has gone:

> "Hey Jarvis" ... (wait for the island to show it is listening, about half a second) ... "Set the volume to 30 percent"

Then mark the time, so the summary ignores everything before it:

```powershell
$from = (Get-Date).ToString("s")
```

## The 20 action-path commands (10 English, 10 Spanish)

For each: say **"Hey Jarvis"**, wait for the listening pose, say the command, wait for the answer to
finish and the island to hide (about 3 s) before the next one. Say each command ONCE; if you
stumble or the room is interrupted, do not repeat it as a "better" sample: write the number down and carry on.
(The commands are the ones the E1 eval already checks: `eval/commands.yaml`.)

| # | Language | Say | Expected |
|---|---|---|---|
| 1 | en | "Open Notepad" | Notepad opens |
| 2 | en | "Set the volume to 30 percent" | volume 30 |
| 3 | en | "Turn the volume up by 10" | volume 40 |
| 4 | en | "Lower the volume a bit" | volume goes down |
| 5 | en | "Mute the sound" | muted (the answer is spoken muted: it is measured but you will not hear it) |
| 6 | en | "Unmute" | unmuted |
| 7 | en | "Open the calculator" | Calculator opens |
| 8 | en | "Set the volume to 40 percent" | volume 40 |
| 9 | en | "Turn the volume up by 10" | volume 50 |
| 10 | en | "Set the volume to 30 percent" | volume 30 |
| 11 | es | "Abrí el bloc de notas" | Notepad opens |
| 12 | es | "Poné el volumen al 30" | volume 30 |
| 13 | es | "Subí el volumen un poco" | volume goes up |
| 14 | es | "Bajá el volumen 20" | volume goes down by 20 |
| 15 | es | "Silenciá la compu" | muted |
| 16 | es | "Sacá el silencio" | unmuted |
| 17 | es | "Abrime la calculadora, por favor" | Calculator opens |
| 18 | es | "Poné el volumen al 40" | volume 40 |
| 19 | es | "Subí el volumen un poco" | volume goes up |
| 20 | es | "Poné el volumen al 30" | volume 30 |

"List my reminders" is NOT in the list: reminders are not built yet (slice 4), so that command is
answered with "not available" and would be labelled `error`.

## The 3 search commands (reported apart, no pass/fail)

Same routine. They take longer by design (a web call plus a second model call) and are never mixed
into the action-path mean.

| # | Language | Say |
|---|---|---|
| S1 | en | "Search for the capital of Australia" |
| S2 | es | "Buscá quién ganó el mundial de 2022" |
| S3 | en | "What's the weather like in Buenos Aires right now?" |

## Read the result

Quit Niva from the tray ("Quit"), then:

```powershell
cd D:\trabajo\ProyectoAsistente
uv run --offline --no-project python tools\latency_summary.py $env:NIVA_LATENCY_LOG --since $from
```

(`--last 23` also works: 20 + 3 turns. `--until <ISO time>` ends a range. The tool also reads the rolled
`.1` file unless you pass `--no-rotated`, and counts only `release` records unless you pass `--all-builds`.)
The exit code is 0 for PASS, 1 for FAIL, 2 when there is no verdict (no data, or fewer than 20 action-path samples).

What to look at:

- **`ACTION PATH: PASS/FAIL`** is judged on `n` voice turns labelled `action` that made a sound:
  mean < 2600 ms and p95 <= 3000 ms (nearest-rank; with 20 turns the p95 is the 19th fastest).
  Below 20 samples there is no verdict (`NOT ENOUGH DATA`, exit 2). An action turn that made NO sound
  (voice down, a failed speech) is a FAIL by itself, with a warning naming how many: it is not in the
  mean or p95, but a command that got no answer does not meet the goal. If it is lower, some turns got another label: see the `turns` column (everything
  logged for that label) and the `no sound / typed` column, and the other rows (`error`, `cancelled`,
  `followup`). A command the model understood differently (a confirmation, a follow-up question) is
  a recognition problem to report, not a latency sample.
- **Stage means**: where the time goes. The estimate to compare against is stt ~735 (the VAD silence
  wait ~532 plus Whisper ~203), llm ~1100, tool ~100, tts ~900. A large `other` means time is lost
  between stages (queues, the island, the pipe), not in a model.
- The per-turn lines are in the file: `Get-Content $env:NIVA_LATENCY_LOG -Tail 3` (numbers and words only).

Save the summary output with the date in `docs/` or in the task notes before closing 3.5. Task 3.5
stays open until the owner has run this on the real microphone and recorded the verdict.
