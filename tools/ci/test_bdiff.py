"""Offline planted regressions, exact-time alignment, cache and replay contracts."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import sys
from unittest.mock import patch

from tools.ci import bdiff, bisect_runs


class BehaviourTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def dump(self, name, rate=60, bad=None, end=None):
        records = [{"k": "header", "format": "state-1", "tick_rate": rate}]
        for f in range((end if end is not None else rate) + 1):
            records.append({"k": "snap", "f": f, "seed": [1, 2], "units": [
                {"ut": 0, "g": 7, "hp": 5 if f == bad else 10, "x": 4},
                {"ut": 1, "g": 2, "hp": 20}]})
        path = self.root / name
        path.write_text("\n".join(json.dumps(r) for r in records), encoding="utf-8")
        return path

    def test_planted_difference_reports_first_tick_unit_field_context_and_ignore(self):
        a, b = self.dump("a"), self.dump("b", bad=30)
        diff = bdiff.compare(a, b)
        self.assertEqual((diff["tick_a"], diff["unit"], diff["field"], diff["a"], diff["b"]),
                         (30, [0, 7], "hp", 10, 5))
        self.assertEqual([r["time"] for r in diff["context"]], ["7/15", "29/60", "1/2", "31/60", "8/15"])
        self.assertTrue(bdiff.compare(a, b, ["hp"])["same"])
        self.assertEqual(bdiff.main([str(a), str(b), "--dumps", "--out", str(self.root / "report")]), 1)

    def test_equal_game_time_uses_exact_rationals_and_checks_duration(self):
        a, b = self.dump("a", 25), self.dump("b", 60)
        with self.assertRaisesRegex(ValueError, "different tick rates"):
            bdiff.compare(a, b)
        self.assertEqual(bdiff.compare(a, b, equal_time=True)["compared"], 6)
        b = self.dump("b", 60, bad=24)
        self.assertEqual(bdiff.compare(a, b, equal_time=True)["time"], "2/5")
        self.assertFalse(bdiff.compare(a, self.dump("short", 60, end=59), equal_time=True)["same"])
        # state-dump writes tick 1 onward, rather than a tick-zero snapshot.
        for path in (a, b):
            lines = path.read_text().splitlines()
            path.write_text("\n".join([lines[0], *lines[2:]]))
        self.assertEqual(bdiff.compare(a, b, equal_time=True)["time"], "2/5")

    def test_reordered_units_same_but_added_field_and_missing_ticks_are_detected(self):
        a, b = self.dump("a"), self.dump("b")
        lines = [json.loads(x) for x in b.read_text().splitlines()]
        for r in lines[1:]:
            r["units"].reverse()
        b.write_text("\n".join(json.dumps(r) for r in lines))
        self.assertTrue(bdiff.compare(a, b)["same"])
        lines[10]["units"][0]["new"] = 1
        b.write_text("\n".join(json.dumps(r) for r in lines))
        self.assertEqual(bdiff.compare(a, b)["field"], "new")
        del lines[5]
        b.write_text("\n".join(json.dumps(r) for r in lines))
        with self.assertRaisesRegex(ValueError, "incomplete dump"):
            bdiff.compare(a, b)

    def test_malformed_and_duplicate_dumps_are_errors(self):
        path = self.root / "invalid.jsonl"
        for text in ("[]", '{"format":"state-1","tick_rate":0}',
                     '{"format":"state-1","tick_rate":60}\n{"k":"snap","f":0,"units":[{}]}'):
            path.write_text(text)
            with self.assertRaises(ValueError):
                bdiff.load_dump(path)
        path = self.dump("duplicate")
        lines = path.read_text().splitlines()
        path.write_text("\n".join([*lines, lines[-1]]))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            bdiff.load_dump(path)

    def test_normalize_scenario_preserves_real_time_and_input_durations(self):
        p = self.root / "source.scn"
        p.write_text("scenario 2\ntick_rate 25\nname x\nend 41\nsave hero.d2s\n"
                     "at 1 input hold 3 4 2\nby 3 check player.hp > 0\n"
                     "at 4 input wait-until player.hp > 0 max 5\n")
        text, end = bdiff.normalized_scenario(p)
        self.assertEqual(end, 99)
        self.assertIn("tick_rate 60", text)
        self.assertIn("at 3 input hold 3 4 5", text)
        self.assertIn("by 8 check", text)
        self.assertIn("at 10 input wait-until player.hp > 0 max 12", text)
        self.assertIn(str(self.root / "hero.d2s"), text)
        p.write_text("scenario 2\ntick_rate 60\ntick_rate 25\nname x\nend 1\n")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            bdiff.normalized_scenario(p)

    def fake_game(self, command, **kwargs):
        """Two distinct binaries have identical behaviour; optional bad replay."""
        self.calls.append(command)
        folder = Path(command[command.index("--out") + 1])
        ticks = [int(t) for t in command[command.index("--at") + 1].split(",")]
        end = int(next(l.split()[1] for l in Path(command[2]).read_text().splitlines() if l.startswith("end ")))
        report = {"tick_rate": 60, "ticks": end, "exit": 0, "verdict": "pass", "start_frame": 12,
                  "queries": [{"at": t, "values": {"server.f": t+12, "server.seed": [1, 2],
                              "server.units": [{"ut": 0, "g": 1, "hp":
                                  0 if self.bad_replay and "replay" in str(folder) and t == 1 else 10}]}}
                              for t in ticks]}
        if (getattr(self, "failed_check", False) or
                getattr(self, "replay_failed_check", False) and "replay" in str(folder)):
            report.update(exit=1, verdict="fail", first_failure={"text": "check player.hp > 20", "at": end})
        bdiff.write(folder / "repro.json", report)
        return subprocess.CompletedProcess(command, report["exit"], "", "")

    def test_two_distinct_shas_same_and_replay_detects_nondeterminism(self):
        p = self.root / "case.scn"
        p.write_text("scenario 2\ntick_rate 60\nname x\nend 2\nchar class 0\n")
        self.calls, self.bad_replay = [], False
        shas = ["a" * 40, "b" * 40]
        cache_calls = []
        class FakeCache:
            def command(_, command, **kwargs):
                return command[-1].split("^")[0]
            def get(_, sha, profile):
                cache_calls.append(("get", sha))
                return Path(sha + ".exe")
            def build(_, *args):
                raise AssertionError("cached binaries must not build")
        with patch.object(bdiff, "Cache", FakeCache), patch.object(bdiff.subprocess, "run", self.fake_game):
            args = [*shas, str(p), "--replay", "1", "--out", str(self.root / "same")]
            self.assertEqual(bdiff.main(args), 0)
            self.assertEqual([x[1] for x in cache_calls], shas)
            self.assertEqual(len(self.calls), 4)
            self.bad_replay = True
            args[-1] = str(self.root / "changed")
            self.assertEqual(bdiff.main(args), 1)
            report = bdiff.read(self.root / "changed/bdiff.json")
            self.assertEqual(report["cases"][0]["replays"][0]["field"], "hp")
            self.bad_replay = False
            self.replay_failed_check = True
            args[-1] = str(self.root / "failed-check-replay")
            self.assertEqual(bdiff.main(args), 1)
            report = bdiff.read(self.root / "failed-check-replay/bdiff.json")
            self.assertEqual(report["cases"][0]["replays"][0]["field"], "repro.first_failure")

    def test_cache_miss_only_builds_via_q1(self):
        class FakeCache:
            def command(_, *args, **kwargs):
                return "a" * 40
            def get(_, *args):
                raise FileNotFoundError()
            def build(_, sha, profile):
                return (sha, profile)
        self.assertEqual(bdiff.binary("main", FakeCache()), ("a" * 40, "release"))

    def test_cli_entrypoints_and_sibling_random_import_work(self):
        for name in ("bdiff.py", "bisect_runs.py"):
            result = subprocess.run([sys.executable, str(Path(bdiff.__file__).with_name(name)), "--help"],
                                    capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stderr)
        result = subprocess.run([sys.executable, "-c",
                                 "import bisect, pathlib, statistics, sysconfig; "
                                 "assert pathlib.Path(bisect.__file__).resolve() == "
                                 "(pathlib.Path(sysconfig.get_path('stdlib')) / 'bisect.py').resolve(); "
                                 "print(statistics.median([1,3]))"],
                                cwd=Path(bdiff.__file__).parent, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_query_chunks_cover_all_ticks_and_recheck_overlap(self):
        p = self.root / "long.scn"
        p.write_text("scenario 2\ntick_rate 60\nname x\nend 501\n")
        self.calls, self.bad_replay = [], False
        with patch.object(bdiff.subprocess, "run", self.fake_game):
            dump = bdiff.run_scenario("fake", p, self.root / "chunks")
        self.assertEqual(len(bdiff.load_dump(dump)[1]), 502)
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.calls[1][-1], "0,499,500,501")

    def test_sweep_generates_once_then_replays_identical_cases(self):
        self.calls, self.bad_replay = [], False
        def game(command, **kwargs):
            if command[1] == "sweep":
                self.calls.append(command)
                folder = Path(command[command.index("--out")+1]) / "scenarios"
                folder.mkdir(parents=True)
                (folder / "skills-case.scn").write_text("scenario 2\ntick_rate 60\nname x\nend 2\n")
                bdiff.write(folder.parent / "sweep.json", {"format": "sweep 1", "exit": 0})
                return subprocess.CompletedProcess(command, 0, "", "")
            return self.fake_game(command, **kwargs)
        with patch.object(bdiff, "binary", lambda value, **kwargs: value), patch.object(bdiff.subprocess, "run", game):
            self.assertEqual(bdiff.main(["build-a", "build-b", "--sweep", "skills", "--out", str(self.root / "sweep")]), 0)
        self.assertEqual([c[1] for c in self.calls], ["sweep", "repro", "repro"])
        self.assertEqual(self.calls[0][0], "build-b")
        self.assertEqual(Path(self.calls[1][2]).read_text(), Path(self.calls[2][2]).read_text())

    def test_missing_report_and_nonzero_exit_are_errors(self):
        p = self.root / "x.scn"
        p.write_text("scenario 2\ntick_rate 60\nname x\nend 1\n")
        with patch.object(bdiff.subprocess, "run", return_value=subprocess.CompletedProcess([], 3, "", "missing data")):
            with self.assertRaisesRegex(RuntimeError, "exited 3"):
                bdiff.run_scenario("fake", p, self.root / "error")

    def test_completed_failed_check_is_a_bad_verdict_with_full_trace(self):
        p = self.root / "x.scn"
        p.write_text("scenario 2\ntick_rate 60\nname x\nend 2\n")
        self.calls, self.bad_replay, self.failed_check = [], False, True
        with patch.object(bdiff, "binary", lambda value, **kwargs: value), patch.object(bdiff.subprocess, "run", self.fake_game):
            self.assertEqual(bdiff.main(["a", "b", str(p), "--out", str(self.root / "failed")]), 1)
        report = bdiff.read(self.root / "failed/bdiff.json")
        self.assertEqual(len(report["cases"][0]["failures"]), 2)
        self.assertTrue((self.root / "failed/case-0000/b/state.jsonl").is_file())

    def test_perf_median_and_threshold(self):
        p = self.root / "p.json"
        bdiff.write(p, {"summary": {"phases_us": {"server": {"p95": 120}}}})
        self.assertEqual(bdiff.perf_value(p), 120)
        values = iter([100, 10000, 102])
        def run(cmd, out, *args, **kwargs):
            bdiff.write(Path(out) / "perf.json", {"summary": {"phases_us": {"server": {"p95": next(values)}}}})
        with patch.object(bdiff, "run_command", run):
            sample = bdiff.perf_sample("fake", "case.scn", self.root / "perf", None, 10, 3, "server.p95")
        self.assertEqual(sample["median_us"], 102)
        self.assertEqual(sample["samples_us"], [100, 10000, 102])
        def sample(exe, *args):
            value = 100 if exe == "a" else 120
            return {"median_us": value, "samples_us": [value] * 3}
        scenario = self.root / "case.scn"
        scenario.write_text("scenario 2\ntick_rate 60\nname x\nend 1\n")
        with patch.object(bdiff, "binary", lambda value, **kwargs: value), patch.object(bdiff, "perf_sample", sample):
            self.assertEqual(bdiff.main(["a", "b", str(scenario), "--perf", "--out", str(self.root / "trend")]), 1)
        trend = bdiff.read(self.root / "trend/bdiff.json")
        self.assertEqual([b["input"] for b in trend["builds"]], ["a", "b"])
        self.assertEqual(trend["cases"][0]["threshold_percent"], 10)
        self.assertEqual(trend["cases"][0]["change_percent"], 20)


class BisectTests(unittest.TestCase):
    def test_synthetic_git_history_finds_planted_bad_commit(self):
        with tempfile.TemporaryDirectory() as tmp:
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=tmp, text=True, stderr=subprocess.PIPE).strip()
            git("init", "-q")
            git("config", "user.name", "Synthetic Test")
            git("config", "user.email", "test@example.invalid")
            commits = []
            for i in range(9):
                Path(tmp, "state").write_text("bad" if i >= 5 else "good")
                git("add", "state")
                git("commit", "-q", "--allow-empty", "-m", f"synthetic {i}")
                commits.append(git("rev-parse", "HEAD"))
            ordered = bisect_runs.history(tmp, commits[0], commits[-1])
            self.assertEqual(ordered, commits)
            result = bisect_runs.search(ordered, lambda sha: git("show", f"{sha}:state") == "bad")
            self.assertEqual(result["first_bad"], commits[5])
            self.assertEqual(result["previous_good"], commits[4])
            self.assertLessEqual(len(result["probes"]), 6)
            scenario = Path(tmp, "case.scn")
            scenario.write_text("scenario 2\ntick_rate 60\nname x\nend 1\n")
            def recorded(sha, scenario, folder, *args):
                folder.mkdir(parents=True)
                hp = 0 if git("show", f"{sha}:state") == "bad" else 10
                records = [{"k": "header", "format": "state-1", "tick_rate": 60},
                           {"k": "snap", "f": 0, "seed": [1, 2], "units": [{"ut": 0, "g": 1, "hp": hp}]}]
                dump = folder / "state.jsonl"
                dump.write_text("\n".join(json.dumps(r) for r in records))
                bdiff.write(folder / "run.json", {"exit": 0})
                return dump
            with patch.object(bdiff, "binary", lambda sha, cache: sha), patch.object(bdiff, "run_scenario", recorded):
                code = bisect_runs.main([str(scenario), "--good", commits[0], "--bad", commits[-1],
                                    "--repo", tmp, "--out", str(Path(tmp, "bisect"))])
            self.assertEqual(code, 0)
            self.assertEqual(bdiff.read(Path(tmp, "bisect/bisect.json"))["first_bad"], commits[5])
            with self.assertRaises(ValueError):
                bisect_runs.history(tmp, commits[-1], commits[0])

    def test_perf_bisect_and_invalid_endpoints_or_crash(self):
        commits = list(range(16))
        timings = [100] * 7 + [130] * 9
        result = bisect_runs.search(commits, lambda i: timings[i] > timings[0] * 1.1)
        self.assertEqual(result["first_bad"], 7)
        with self.assertRaisesRegex(ValueError, "bad endpoint is good"):
            bisect_runs.search(commits, lambda _: False)
        with self.assertRaisesRegex(ValueError, "good endpoint is bad"):
            bisect_runs.search(commits, lambda _: True)
        def crash(i):
            if i == 7:
                raise RuntimeError("crash is not a behaviour verdict")
            return i > 6
        with self.assertRaises(RuntimeError):
            bisect_runs.search(commits, crash)


if __name__ == "__main__":
    unittest.main()
