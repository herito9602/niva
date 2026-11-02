# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = [
#   "faster-whisper",
#   "nvidia-cublas-cu12",
#   "nvidia-cudnn-cu12",
#   "edge-tts",
#   "miniaudio",
#   "numpy",
#   "ollama",
#   "pyyaml",
# ]
# ///
"""S1: can faster-whisper share the RTX 3060 Laptop (6 GB) with the product LLM?

Gates (openspec/changes/niva-mvp/design.md, Spikes S1): STT p95 <= 400 ms including es/en
language detection, language-ID >= 19/20, and no CPU spill of the LLM when both are resident.

Audio is SYNTHETIC (edge-tts) - there are no real microphone recordings yet.

Usage:
  uv run spikes/voice/s1_gpu_stt.py --model small --mode solo
  uv run spikes/voice/s1_gpu_stt.py --model large-v3-turbo --mode co
Modes:
  solo  Whisper only (Ollama models are unloaded first).
  co    Warm the LLM in Ollama, then load Whisper, run clips, then run the E1 eval as a
        subprocess while Whisper is still resident.
"""

import argparse
import asyncio
import gc
import json
import os
import re
import statistics
import subprocess
import sys
import time
import unicodedata
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
AUDIO_DIR = HERE / "s1_audio"
OUT_DIR = HERE
LLM = "qwen3.5:4b-q4_K_M"


def add_nvidia_dll_dirs() -> list[str]:
    """ctranslate2 on Windows needs cuBLAS/cuDNN DLLs; the pip wheels ship them in nvidia/*/bin."""
    import importlib.util

    added = []
    spec = importlib.util.find_spec("nvidia")
    if spec is None or not spec.submodule_search_locations:
        return added
    for base in spec.submodule_search_locations:
        for bin_dir in sorted(Path(base).glob("*/bin")):
            os.add_dll_directory(str(bin_dir))
            os.environ["PATH"] = str(bin_dir) + os.pathsep + os.environ.get("PATH", "")
            added.append(str(bin_dir))
    return added


# ---------------------------------------------------------------- test audio

VOICES = {
    ("es", "f"): "es-AR-ElenaNeural",
    ("es", "m"): "es-AR-TomasNeural",
    ("en", "f"): "en-US-AriaNeural",
    ("en", "m"): "en-US-GuyNeural",
}


def pick_clips() -> list[dict]:
    cases = yaml.safe_load((ROOT / "eval" / "commands.yaml").read_text(encoding="utf-8"))["cases"]
    clips = []
    for lang in ("es", "en"):
        utts = []
        for c in cases:
            if c["lang"] == lang and c["utterance"] not in utts:
                utts.append(c["utterance"])
        # spread evenly across the file so tool categories are mixed
        step = len(utts) / 10
        chosen = [utts[int(i * step)] for i in range(10)]
        for i, text in enumerate(chosen):
            gender = "m" if i in (3, 8) else "f"
            clips.append({"lang": lang, "text": text, "voice": VOICES[(lang, gender)], "id": f"{lang}{i:02d}"})
    return clips


def synthesize(clips: list[dict]) -> None:
    import edge_tts

    AUDIO_DIR.mkdir(exist_ok=True)

    async def one(clip):
        mp3 = AUDIO_DIR / f"{clip['id']}.mp3"
        if not mp3.exists():
            await edge_tts.Communicate(clip["text"], clip["voice"]).save(str(mp3))
        clip["mp3"] = mp3

    async def run():
        for clip in clips:
            await one(clip)

    asyncio.run(run())


def decode(clip: dict):
    import miniaudio
    import numpy as np

    d = miniaudio.decode_file(str(clip["mp3"]), output_format=miniaudio.SampleFormat.SIGNED16, nchannels=1, sample_rate=16000)
    return np.frombuffer(d.samples, dtype=np.int16).astype(np.float32) / 32768.0


# ---------------------------------------------------------------- helpers


def norm(s: str) -> list[str]:
    s = unicodedata.normalize("NFD", s.lower())
    s = "".join(ch for ch in s if unicodedata.category(ch) != "Mn")
    return re.findall(r"[a-z0-9]+", s)


def word_hit(ref: str, hyp: str) -> float:
    r, h = norm(ref), set(norm(hyp))
    return sum(1 for w in r if w in h) / len(r) if r else 0.0


def pct(vals, p):
    vals = sorted(vals)
    k = (len(vals) - 1) * p
    lo, hi = int(k), min(int(k) + 1, len(vals) - 1)
    return vals[lo] + (vals[hi] - vals[lo]) * (k - lo)


def sh(cmd: list[str]) -> str:
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace").stdout.strip()


def gpu_mem() -> dict:
    out = sh(["nvidia-smi", "--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits"])
    used, total = [int(x) for x in out.split(",")]
    return {"used_mib": used, "total_mib": total}


def ollama_ps() -> str:
    return sh(["ollama", "ps"])


def load_whisper(name: str):
    from faster_whisper import WhisperModel

    errors = []
    for ct in ("float16", "int8_float16"):
        try:
            m = WhisperModel(name, device="cuda", compute_type=ct)
            return m, ct, errors
        except Exception as e:  # noqa: BLE001 - spike: report whatever ctranslate2 raises
            errors.append(f"{ct}: {type(e).__name__}: {e}")
    raise RuntimeError("could not load Whisper: " + " | ".join(errors))


def detect_es_en(model, audio):
    """Language probabilities from the installed faster-whisper; pick the higher of es vs en."""
    _lang, _p, all_probs = model.detect_language(audio)
    probs = dict(all_probs)
    es, en = probs.get("es", 0.0), probs.get("en", 0.0)
    return ("es" if es >= en else "en"), es, en


def stt_one(model, audio):
    t0 = time.perf_counter()
    lang, p_es, p_en = detect_es_en(model, audio)
    t1 = time.perf_counter()
    segs, _info = model.transcribe(
        audio, language=lang, beam_size=1, vad_filter=False, condition_on_previous_text=False, temperature=0.0
    )
    text = "".join(s.text for s in segs).strip()
    t2 = time.perf_counter()
    return {
        "lang": lang,
        "p_es": p_es,
        "p_en": p_en,
        "text": text,
        "detect_ms": (t1 - t0) * 1000,
        "total_ms": (t2 - t0) * 1000,
    }


def run_clips(model, clips, audios) -> dict:
    rows = []
    for clip, audio in zip(clips, audios):
        r = stt_one(model, audio)
        r.update(id=clip["id"], ref=clip["text"], ref_lang=clip["lang"], voice=clip["voice"], dur_s=len(audio) / 16000)
        r["lang_ok"] = r["lang"] == clip["lang"]
        r["word_hit"] = word_hit(clip["text"], r["text"])
        rows.append(r)
    lat = [r["total_ms"] for r in rows]
    return {
        "rows": rows,
        "lat_mean": statistics.mean(lat),
        "lat_p50": pct(lat, 0.5),
        "lat_p95": pct(lat, 0.95),
        "lat_max": max(lat),
        "lang_ok": sum(r["lang_ok"] for r in rows),
        "mean_word_hit": statistics.mean(r["word_hit"] for r in rows),
    }


def print_clips(res: dict) -> None:
    for r in res["rows"]:
        flag = "OK " if r["lang_ok"] else "BAD"
        print(
            f"  [{r['id']}] {flag} lang={r['lang']} (es={r['p_es']:.2f} en={r['p_en']:.2f}) "
            f"{r['total_ms']:6.0f} ms  hit={r['word_hit']:.2f}  ref={r['ref']!r}  hyp={r['text']!r}"
        )
    print(
        f"  latency ms mean={res['lat_mean']:.0f} p50={res['lat_p50']:.0f} p95={res['lat_p95']:.0f} "
        f"max={res['lat_max']:.0f} | lang-ID {res['lang_ok']}/20 | mean word-hit {res['mean_word_hit']:.2f}"
    )


# ---------------------------------------------------------------- main


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True, help="faster-whisper model name (small, large-v3-turbo)")
    ap.add_argument("--mode", choices=["solo", "co"], required=True)
    args = ap.parse_args()

    dll_dirs = add_nvidia_dll_dirs()
    print("DLL dirs added:", dll_dirs)

    clips = pick_clips()
    synthesize(clips)
    audios = [decode(c) for c in clips]
    print(f"{len(clips)} synthetic clips, total {sum(len(a) for a in audios) / 16000:.1f} s of audio")

    import ollama

    out: dict = {"model": args.model, "mode": args.mode, "dll_dirs": dll_dirs}

    # Start from an empty GPU: unload anything Ollama holds.
    for m in ollama.ps().get("models", []):
        ollama.generate(model=m["model"], prompt="", keep_alive=0)
    time.sleep(2)
    out["gpu_baseline"] = gpu_mem()
    print("baseline GPU:", out["gpu_baseline"])

    if args.mode == "co":
        t0 = time.perf_counter()
        ollama.chat(
            model=LLM,
            messages=[{"role": "user", "content": "Say hi."}],
            think=False,
            options={"num_predict": 8},
            keep_alive="30m",
        )
        out["llm_warm_s"] = time.perf_counter() - t0
        time.sleep(1)
        out["gpu_llm_only"] = gpu_mem()
        out["ps_llm_only"] = ollama_ps()
        print("LLM only:", out["gpu_llm_only"], "\n" + out["ps_llm_only"])

    model, ctype, load_errors = load_whisper(args.model)
    out["compute_type"], out["load_errors"] = ctype, load_errors
    # warm-up (CUDA kernels, cuDNN autotune): a few passes over different clips
    for a in (audios[0], audios[10], audios[1]):
        stt_one(model, a)
    time.sleep(1)
    out["gpu_after_whisper_load"] = gpu_mem()
    print("after Whisper load+warmup:", out["gpu_after_whisper_load"])

    res = run_clips(model, clips, audios)
    out["clips"] = res
    out["gpu_after_clips"] = gpu_mem()
    print(f"== {args.model} ({ctype}) {args.mode} ==")
    print_clips(res)

    if args.mode == "co":
        out["ps_after_whisper"] = ollama_ps()
        print("ollama ps with Whisper resident:\n" + out["ps_after_whisper"])
        # E1 eval with Whisper still resident in this process
        p = subprocess.run(
            ["uv", "run", str(ROOT / "spikes" / "eval" / "run_e1.py"), "--model", LLM],
            capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=str(ROOT),
        )
        out["e1_stdout"] = p.stdout
        out["e1_returncode"] = p.returncode
        print(p.stdout[-2500:])
        if p.returncode:
            print(p.stderr[-2000:])
        out["gpu_after_e1"] = gpu_mem()
        out["ps_after_e1"] = ollama_ps()
        print("after E1:", out["gpu_after_e1"], "\n" + out["ps_after_e1"])
        # does Whisper still meet latency while the LLM is resident and was just hammered?
        res2 = run_clips(model, clips, audios)
        out["clips_after_e1"] = res2
        print("-- clips again after E1 --")
        print(
            f"  latency ms mean={res2['lat_mean']:.0f} p50={res2['lat_p50']:.0f} p95={res2['lat_p95']:.0f} "
            f"| lang-ID {res2['lang_ok']}/20"
        )

    del model
    gc.collect()
    for m in ollama.ps().get("models", []):
        ollama.generate(model=m["model"], prompt="", keep_alive=0)

    path = OUT_DIR / f"s1_raw_{args.model}_{args.mode}.json"
    path.write_text(json.dumps(out, ensure_ascii=False, indent=2), encoding="utf-8")
    print("saved", path)


if __name__ == "__main__":
    sys.exit(main())
