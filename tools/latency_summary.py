"""Summarise the Niva per-turn latency log (task 3.5).

Reads the JSON Lines file the app writes (see docs/latency-e2e.md), filters it
by time range or by the last N records, and prints, per path label, the count
and the mean, p50, p95 and max of "end of speech -> first audio" (`total_ms`),
plus the mean of each stage. It then judges the ACTION path against the
speech-io goals:

    mean < 2600 ms   and   p95 <= 3000 ms

Percentiles use the nearest-rank method: p95 of n samples is the ceil(0.95*n)-th
smallest. Only voice turns that made a sound have a `total_ms`; the rest are
counted and left out of the statistics (they are not invented).

Standard library only (Python 3.8+). Exit code: 0 PASS, 1 FAIL, 2 no data or bad
input. Run it with: uv run --offline --no-project python tools\\latency_summary.py
"""

import argparse
import datetime
import json
import math
import os
import re
import sys
from typing import Dict, Iterable, List, Optional, Tuple

MEAN_LIMIT_MS = 2600.0
P95_LIMIT_MS = 3000
MIN_SAMPLES = 20
STAGES = ("stt_ms", "llm_ms", "tool_ms", "tts_ms", "other_ms")
# Reported in this order; any other label is appended after them.
PATH_ORDER = ("action", "search", "chat", "confirm", "followup", "sleep", "error", "cancelled")
ACTION = "action"


def percentile(sorted_values: List[int], pct: float) -> Optional[int]:
    """Nearest-rank percentile of an ascending list; None when it is empty."""
    if not sorted_values:
        return None
    rank = max(1, math.ceil(pct / 100.0 * len(sorted_values)))
    return sorted_values[min(rank, len(sorted_values)) - 1]


def mean(values: Iterable[int]) -> Optional[float]:
    values = list(values)
    return sum(values) / len(values) if values else None


def stats(values: List[int]) -> Dict[str, Optional[float]]:
    ordered = sorted(values)
    return {
        "count": len(ordered),
        "mean": mean(ordered),
        "p50": percentile(ordered, 50),
        "p95": percentile(ordered, 95),
        "max": ordered[-1] if ordered else None,
    }


def verdict(values: List[int]) -> Tuple[str, str]:
    """('PASS' | 'FAIL' | 'NO DATA', the reason in one line) for the action path."""
    if not values:
        return "NO DATA", "no action-path voice turn with a first sound in the selection"
    s = stats(values)
    mean_ok = s["mean"] < MEAN_LIMIT_MS
    p95_ok = s["p95"] <= P95_LIMIT_MS
    detail = "mean {:.0f} ms ({} {:.0f}), p95 {} ms ({} {})".format(
        s["mean"],
        "<" if mean_ok else ">=",
        MEAN_LIMIT_MS,
        s["p95"],
        "<=" if p95_ok else ">",
        P95_LIMIT_MS,
    )
    return ("PASS" if mean_ok and p95_ok else "FAIL"), detail


def parse_when(text: str, now: Optional[datetime.datetime] = None) -> int:
    """A moment as epoch milliseconds: '90m', '2h', '1d' (ago) or an ISO time.

    An ISO time without a zone is read as local time.
    """
    now = now or datetime.datetime.now()
    ago = re.fullmatch(r"(\d+)([mhd])", text.strip())
    if ago:
        unit = {"m": 60, "h": 3600, "d": 86400}[ago.group(2)]
        moment = now - datetime.timedelta(seconds=int(ago.group(1)) * unit)
    else:
        moment = datetime.datetime.fromisoformat(text.strip())
    try:
        return int(moment.timestamp() * 1000)
    except (OSError, OverflowError) as err:  # e.g. before 1970 on Windows
        raise ValueError("time out of range: {}".format(text)) from err


def read_records(paths: Iterable[str]) -> Tuple[List[dict], int]:
    """Every record of the files, in order, and the number of lines skipped."""
    records: List[dict] = []
    skipped = 0
    for path in paths:
        with open(path, "r", encoding="utf-8", errors="replace") as handle:
            for line in handle:
                line = line.strip()
                if not line:
                    continue
                try:
                    record = json.loads(line)
                except ValueError:
                    skipped += 1
                    continue
                if isinstance(record, dict) and isinstance(record.get("path"), str):
                    records.append(record)
                else:
                    skipped += 1
    return records, skipped


def select(
    records: List[dict],
    since_ms: Optional[int] = None,
    until_ms: Optional[int] = None,
    last: Optional[int] = None,
    release_only: bool = False,
) -> List[dict]:
    """The time range and build filters, then the last N. A record without a
    `build` field counts as a release one."""
    chosen = [
        r
        for r in records
        if (not release_only or r.get("build", "release") == "release")
        and (since_ms is None or r.get("at", 0) >= since_ms)
        and (until_ms is None or r.get("at", 0) <= until_ms)
    ]
    if last is not None:
        chosen = chosen[-last:] if last > 0 else []
    return chosen


def group(records: List[dict]) -> Dict[str, dict]:
    """Per path label: the voice turns' totals, stage values and the counts."""
    out: Dict[str, dict] = {}
    for r in records:
        entry = out.setdefault(
            r["path"],
            {"turns": 0, "text": 0, "silent": 0, "totals": [], "stages": {s: [] for s in STAGES}},
        )
        entry["turns"] += 1
        if r.get("channel") != "voice":
            entry["text"] += 1
            continue
        total = r.get("total_ms")
        if not isinstance(total, int):
            entry["silent"] += 1
            continue
        entry["totals"].append(total)
        for stage in STAGES:
            if isinstance(r.get(stage), int):
                entry["stages"][stage].append(r[stage])
    return out


def fmt(value: Optional[float]) -> str:
    return "-" if value is None else "{:.0f}".format(value)


def render(records: List[dict], skipped: int = 0, excluded: int = 0) -> Tuple[str, int]:
    """The report text and the exit code."""
    groups = group(records)
    labels = [p for p in PATH_ORDER if p in groups] + sorted(
        p for p in groups if p not in PATH_ORDER
    )
    lines = []
    lines.append(
        "Latency log: {} record(s){}".format(
            len(records), ", {} unreadable line(s) skipped".format(skipped) if skipped else ""
        )
    )
    if excluded:
        lines.append(
            "{} debug/other-build record(s) left out (--all-builds includes them)".format(excluded)
        )
    lines.append("end of speech -> first audio (ms), voice turns that made a sound")
    lines.append("")
    lines.append(
        "{:<10} {:>5} {:>6} {:>7} {:>7} {:>7} {:>7}   {}".format(
            "path", "turns", "n", "mean", "p50", "p95", "max", "no sound / typed"
        )
    )
    for label in labels:
        g = groups[label]
        s = stats(g["totals"])
        lines.append(
            "{:<10} {:>5} {:>6} {:>7} {:>7} {:>7} {:>7}   {} / {}".format(
                label,
                g["turns"],
                s["count"],
                fmt(s["mean"]),
                fmt(s["p50"]),
                fmt(s["p95"]),
                fmt(s["max"]),
                g["silent"],
                g["text"],
            )
        )
    lines.append("")
    lines.append("stage means (ms)")
    lines.append(
        "{:<10} {:>7} {:>7} {:>7} {:>7} {:>7}".format("path", "stt", "llm", "tool", "tts", "other")
    )
    for label in labels:
        stages = groups[label]["stages"]
        lines.append(
            "{:<10} {}".format(
                label, " ".join("{:>7}".format(fmt(mean(stages[s]))) for s in STAGES)
            )
        )
    lines.append("")
    entry = groups.get(ACTION, {"totals": [], "silent": 0})
    action, silent = entry["totals"], entry["silent"]
    status, detail = verdict(action)
    if silent:
        # A voice command that never got an answer is a failure of the goal,
        # whatever the mean of the others says.
        status = "FAIL"
        detail = "{} action turn(s) made no sound; {}".format(silent, detail)
    elif action and len(action) < MIN_SAMPLES:
        status = "NOT ENOUGH DATA"
        detail = "only {} action-path samples, need at least {}; {}".format(
            len(action), MIN_SAMPLES, detail
        )
    lines.append("ACTION PATH: {}  ({})".format(status, detail))
    lines.append(
        "goal: mean < {:.0f} ms and p95 <= {} ms over at least {} action-path voice commands, "
        "none of them silent".format(MEAN_LIMIT_MS, P95_LIMIT_MS, MIN_SAMPLES)
    )
    if silent:
        lines.append(
            "warning: {} action turn(s) made no sound (voice down, a failed speech, or a "
            "stopped one). They are not in the mean or p95 above, and each one fails the "
            "goal.".format(silent)
        )
    if "search" in groups:
        lines.append("search path is reported apart and has no pass/fail here")
    code = {"PASS": 0, "FAIL": 1}.get(status, 2)
    return "\n".join(lines) + "\n", code


def default_path() -> Optional[str]:
    env = os.environ.get("NIVA_LATENCY_LOG")
    if env:
        return env
    base = os.environ.get("LOCALAPPDATA")
    return os.path.join(base, "Niva", "latency.jsonl") if base else None


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Summarise the Niva latency log.")
    parser.add_argument(
        "file",
        nargs="?",
        help="log file (default: NIVA_LATENCY_LOG, else %%LOCALAPPDATA%%\\Niva\\latency.jsonl)",
    )
    parser.add_argument(
        "--since", help="only records from then: '90m', '2h', '1d' ago, or an ISO time (local)"
    )
    parser.add_argument("--until", help="only records up to then (same formats)")
    parser.add_argument("--last", type=int, help="only the last N records (after --since/--until)")
    parser.add_argument(
        "--all-builds",
        action="store_true",
        help="also count records from debug builds (by default only `release` ones)",
    )
    parser.add_argument(
        "--no-rotated",
        action="store_true",
        help="do not read '<file>.1' (the previous, rolled-over file)",
    )
    args = parser.parse_args(argv)

    path = args.file or default_path()
    if not path:
        print("no log file: pass one, or set NIVA_LATENCY_LOG / LOCALAPPDATA", file=sys.stderr)
        return 2
    files = []
    if not args.no_rotated and os.path.exists(path + ".1"):
        files.append(path + ".1")
    if os.path.exists(path):
        files.append(path)
    if not files:
        print("no such log file: {}".format(path), file=sys.stderr)
        return 2
    try:
        since = parse_when(args.since) if args.since else None
        until = parse_when(args.until) if args.until else None
    except (ValueError, OverflowError) as err:
        print("bad time: {}".format(err), file=sys.stderr)
        return 2
    try:
        records, skipped = read_records(files)
    except OSError as err:
        print("cannot read the log: {}".format(err), file=sys.stderr)
        return 2
    chosen = select(records, since, until, args.last, release_only=not args.all_builds)
    # Counted on the same time range, so the note matches what was left out.
    kept = len(select(records, since, until, None, release_only=False)) - len(
        select(records, since, until, None, release_only=not args.all_builds)
    )
    text, code = render(chosen, skipped, kept)
    sys.stdout.write(text)
    return code


if __name__ == "__main__":
    sys.exit(main())
