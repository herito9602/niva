"""Tests of tools/latency_summary.py. Run (from the repository root):

    uv run --offline --no-project python -m unittest discover -s tools -v
"""

import contextlib
import datetime
import io
import os
import tempfile
import unittest

import latency_summary as ls

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "fixtures", "latency_sample.jsonl")
BASE = 1700000000000


def action(total, at=BASE, channel="voice"):
    return {"path": "action", "channel": channel, "total_ms": total, "at": at}


class PercentileTests(unittest.TestCase):
    def test_nearest_rank_on_twenty_samples(self):
        values = list(range(1, 21))
        self.assertEqual(ls.percentile(values, 95), 19)
        self.assertEqual(ls.percentile(values, 50), 10)
        self.assertEqual(ls.percentile(values, 100), 20)

    def test_small_samples_and_empty(self):
        self.assertEqual(ls.percentile([7], 95), 7)
        self.assertEqual(ls.percentile([1, 2], 50), 1)
        self.assertEqual(ls.percentile([1, 2], 95), 2)
        self.assertIsNone(ls.percentile([], 95))

    def test_stats(self):
        s = ls.stats([300, 100, 200])
        self.assertEqual((s["count"], s["mean"], s["p50"], s["max"]), (3, 200.0, 200, 300))
        self.assertEqual(ls.stats([])["mean"], None)


class VerdictTests(unittest.TestCase):
    def test_pass_when_both_limits_hold(self):
        status, _ = ls.verdict([2000] * 19 + [3000])
        self.assertEqual(status, "PASS")

    def test_the_p95_limit_is_inclusive(self):
        # 20 samples: p95 is the 19th. Exactly 3000 passes, 3001 fails.
        self.assertEqual(ls.verdict([1000] * 18 + [3000, 9000])[0], "PASS")
        self.assertEqual(ls.verdict([1000] * 18 + [3001, 9000])[0], "FAIL")

    def test_the_mean_limit_is_exclusive(self):
        self.assertEqual(ls.verdict([2599, 2600 - 1])[0], "PASS")
        self.assertEqual(ls.verdict([2600, 2600])[0], "FAIL")

    def test_a_good_mean_does_not_hide_a_bad_p95(self):
        status, detail = ls.verdict([1500] * 17 + [3500] * 3)
        self.assertEqual(status, "FAIL")
        self.assertIn("p95 3500", detail)

    def test_a_good_p95_does_not_hide_a_bad_mean(self):
        self.assertEqual(ls.verdict([2800] * 20)[0], "FAIL")

    def test_no_data_is_not_a_pass(self):
        self.assertEqual(ls.verdict([])[0], "NO DATA")


class SelectionTests(unittest.TestCase):
    def setUp(self):
        self.records = [action(1000, at=BASE + i * 1000) for i in range(10)]

    def test_time_range_is_inclusive(self):
        got = ls.select(self.records, since_ms=BASE + 3000, until_ms=BASE + 5000)
        self.assertEqual([r["at"] for r in got], [BASE + 3000, BASE + 4000, BASE + 5000])

    def test_last_n_is_applied_after_the_time_range(self):
        got = ls.select(self.records, until_ms=BASE + 6000, last=2)
        self.assertEqual([r["at"] for r in got], [BASE + 5000, BASE + 6000])
        self.assertEqual(ls.select(self.records, last=0), [])
        self.assertEqual(len(ls.select(self.records, last=99)), 10)

    def test_relative_and_iso_times(self):
        now = datetime.datetime(2026, 11, 7, 12, 0, 0)
        self.assertEqual(
            ls.parse_when("90m", now), int((now - datetime.timedelta(minutes=90)).timestamp() * 1000)
        )
        self.assertEqual(
            ls.parse_when("2h", now), int((now - datetime.timedelta(hours=2)).timestamp() * 1000)
        )
        self.assertEqual(
            ls.parse_when("2026-11-07T10:30"),
            int(datetime.datetime(2026, 11, 7, 10, 30).timestamp() * 1000),
        )
        with self.assertRaises(ValueError):
            ls.parse_when("yesterday-ish")


class GroupingTests(unittest.TestCase):
    def test_text_turns_and_silent_turns_are_counted_but_not_measured(self):
        records = [
            action(2000),
            action(2200, channel="text"),
            {"path": "action", "channel": "voice", "at": BASE},  # no sound
        ]
        g = ls.group(records)["action"]
        self.assertEqual((g["turns"], g["text"], g["silent"]), (3, 1, 1))
        self.assertEqual(g["totals"], [2000])

    def test_a_confirmation_never_enters_the_action_mean(self):
        records = [action(2000), {"path": "confirm", "channel": "voice", "total_ms": 9000}]
        g = ls.group(records)
        self.assertEqual(g["action"]["totals"], [2000])
        self.assertEqual(g["confirm"]["totals"], [9000])

    def test_stage_values_are_collected_only_when_present(self):
        records = [
            dict(action(2000), stt_ms=200, llm_ms=1000),
            dict(action(2200), stt_ms=300),
        ]
        stages = ls.group(records)["action"]["stages"]
        self.assertEqual(ls.mean(stages["stt_ms"]), 250.0)
        self.assertEqual(ls.mean(stages["llm_ms"]), 1000.0)
        self.assertIsNone(ls.mean(stages["tool_ms"]))


class FixtureTests(unittest.TestCase):
    def test_the_fixture_is_read_and_a_torn_last_line_is_skipped(self):
        records, skipped = ls.read_records([FIXTURE])
        self.assertEqual(len(records), 26)
        self.assertEqual(skipped, 1)

    def test_the_fixture_report(self):
        records, skipped = ls.read_records([FIXTURE])
        text, code = ls.render(records, skipped)
        self.assertEqual(code, 0)
        self.assertIn("ACTION PATH: PASS", text)
        self.assertIn("mean 2340 ms", text)
        self.assertNotIn("warning", text)
        self.assertIn("p95 3000 ms", text)
        self.assertIn("1 unreadable line(s) skipped", text)
        lines = {}
        for line in text.splitlines():
            if line.startswith(("action ", "search ", "confirm ")):
                lines.setdefault(line.split()[0], line.split())  # the first table only
        # action: 21 turns (one without a sound), 20 measured.
        self.assertEqual(lines["action"][1:3], ["20", "20"])
        self.assertEqual(lines["action"][3:7], ["2340", "2000", "3000", "3400"])
        self.assertEqual(lines["search"][1:3], ["3", "3"])
        self.assertEqual(lines["search"][3], "4633")
        self.assertIn("search path is reported apart", text)

    def test_twenty_slow_turns_fail(self):
        text, code = ls.render([action(3500)] * 20)
        self.assertEqual(code, 1)
        self.assertIn("ACTION PATH: FAIL", text)

    def test_no_action_data_exits_with_two(self):
        text, code = ls.render([{"path": "chat", "channel": "voice", "total_ms": 1000}])
        self.assertEqual(code, 2)
        self.assertIn("NO DATA", text)

    def test_fewer_than_twenty_samples_is_not_a_verdict(self):
        text, code = ls.render([action(2000)] * 19)
        self.assertEqual(code, 2)
        self.assertIn("NOT ENOUGH DATA", text)
        self.assertIn("19", text)
        self.assertEqual(ls.render([action(2000)] * 20)[1], 0)

    def test_silent_action_turns_fail_the_verdict_with_a_warning(self):
        # 15 fast turns and 5 that made no sound: the mean alone would PASS.
        silent = {"path": "action", "channel": "voice", "at": BASE}
        text, code = ls.render([action(1500)] * 15 + [silent] * 5)
        self.assertEqual(code, 1)
        self.assertIn("ACTION PATH: FAIL", text)
        self.assertIn("5 action turn(s) made no sound", text)

    def test_one_silent_action_turn_is_enough_to_fail(self):
        silent = {"path": "action", "channel": "voice", "at": BASE}
        self.assertEqual(ls.render([action(1500)] * 25 + [silent])[1], 1)

    def test_silent_typed_action_turns_do_not_count(self):
        typed = {"path": "action", "channel": "text", "at": BASE}
        self.assertEqual(ls.render([action(1500)] * 20 + [typed])[1], 0)

    def test_other_builds_are_left_out_by_default(self):
        debug = dict(action(9000), build="debug")
        rel = dict(action(1500), build="release")
        records = [rel] * 20 + [debug] * 3
        self.assertEqual(len(ls.select(records, release_only=True)), 20)
        self.assertEqual(len(ls.select(records)), 23)
        self.assertEqual(len(ls.select([action(1)], release_only=True)), 1)  # no build field


class CommandLineTests(unittest.TestCase):
    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = ls.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def test_a_file_a_pass(self):
        code, out, _ = self.run_main(FIXTURE)
        self.assertEqual(code, 0)
        self.assertIn("ACTION PATH: PASS", out)

    def test_last_selects_from_the_end(self):
        # The last 3 records of the fixture are confirm, a silent action, a typed chat.
        code, out, _ = self.run_main(FIXTURE, "--last", "3")
        self.assertEqual(code, 2)
        self.assertIn("3 record(s)", out)

    def test_a_missing_file_is_an_error(self):
        code, _, err = self.run_main(os.path.join(HERE, "no-such-file.jsonl"))
        self.assertEqual(code, 2)
        self.assertIn("no such log file", err)

    def test_a_bad_time_is_an_error(self):
        code, _, err = self.run_main(FIXTURE, "--since", "whenever")
        self.assertEqual(code, 2)
        self.assertIn("bad time", err)

    def test_a_huge_relative_time_is_an_error_not_a_crash(self):
        code, _, err = self.run_main(FIXTURE, "--since", "99999999999d")
        self.assertEqual(code, 2)
        self.assertIn("bad time", err)

    def test_an_unreadable_log_is_an_error_not_a_verdict(self):
        with tempfile.TemporaryDirectory() as tmp:
            code, out, err = self.run_main(tmp)  # a directory cannot be read as a log
        self.assertEqual(code, 2)
        self.assertIn("cannot read the log", err)
        self.assertEqual(out, "")

    def test_debug_records_are_excluded_unless_asked(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "latency.jsonl")
            with open(path, "w") as f:
                for _ in range(20):
                    f.write('{"path":"action","channel":"voice","total_ms":1500,"at":1,"build":"release"}\n')
                f.write('{"path":"action","channel":"voice","total_ms":9500,"at":1,"build":"debug"}\n')
            code, out, _ = self.run_main(path)
            self.assertEqual(code, 0)
            self.assertIn("20 record(s)", out)
            self.assertIn("1 debug", out)
            code, out, _ = self.run_main(path, "--all-builds")
            self.assertIn("21 record(s)", out)

    def test_the_rolled_file_is_read_first_unless_asked_not_to(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "latency.jsonl")
            with open(path + ".1", "w") as old:
                old.write('{"path":"action","channel":"voice","total_ms":1000,"at":1}\n')
            with open(path, "w") as new:
                new.write('{"path":"action","channel":"voice","total_ms":2000,"at":2}\n')
            _, out, _ = self.run_main(path)
            self.assertIn("2 record(s)", out)
            _, out, _ = self.run_main(path, "--no-rotated")
            self.assertIn("1 record(s)", out)

    def test_the_time_range_reaches_the_report(self):
        code, out, _ = self.run_main(FIXTURE, "--since", "2001-01-01T00:00", "--until", "2001-01-02T00:00")
        self.assertEqual(code, 2)
        self.assertIn("0 record(s)", out)


if __name__ == "__main__":
    unittest.main()
