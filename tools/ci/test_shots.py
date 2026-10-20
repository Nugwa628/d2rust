"""Picture orchestration tests use fake local executables, never game files."""
import argparse
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import shots


class ShotsTests(unittest.TestCase):
    def spec(self):
        return {"name": "walk", "args": ["--new", "sorceress", "Picture"],
                "ticks": [1, 2], "phases": [0, .5]}

    def test_manifest_validation(self):
        shots.validate_scenario(self.spec())
        for change in ({"name": "../escape"}, {"args": ["play"]},
                       {"args": ["--new", "sorceress"]}, {"ticks": []},
                       {"ticks": [2, 1]}, {"phases": [.5]},
                       {"phases": [0, .001]}, {"size": "0x0"}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                shots.validate_scenario(dict(self.spec(), **change))

    def test_phase_milliseconds_follow_manifest_rate(self):
        self.assertEqual(shots.validate_scenario(self.spec())[-1], [0, 20])
        self.assertEqual(shots.validate_scenario(dict(self.spec(), tick_rate=60))[-1], [0, 8])
        for rate in (0, 1001, True, "60"):
            with self.subTest(rate=rate), self.assertRaises(ValueError):
                shots.validate_scenario(dict(self.spec(), tick_rate=rate))

    def test_capture_hidden_fixed_clock_and_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            exe = root / "game.exe"
            exe.write_bytes(b"fake game")
            source = root / "suite.json"
            shots.write(source, self.spec())
            options = argparse.Namespace(sha="main", bin=str(exe), scenarios=str(source),
                root=root / "golden", label=None, repeats=2, game_dir="local-game", timeout=10)
            commands = []

            def fake_run(command, **kwargs):
                commands.append(command)
                self.assertIn("--offscreen", command)
                self.assertEqual(command[command.index("--display-clock") + 1], "phases:0,0.5")
                self.assertIn("D2RUST_HOME", kwargs["env"])
                folder = Path(command[command.index("--shot-dir") + 1])
                phase = command[command.index("--shot-phase") + 1]
                for tick in (1, 2):
                    (folder / (f"tick-{tick:06}" + ("-f1" if phase == "0.5" else "") + ".png")).write_bytes(b"png")

            with patch.object(shots, "commit", return_value="abc"), patch.object(shots.subprocess, "run", side_effect=fake_run), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(shots.capture(options), 0)
            result = shots.read(options.root / "abc/capture.json")
            self.assertTrue(result["complete"])
            self.assertEqual(len(commands), 4)
            self.assertEqual(len(result["scenarios"][0]["frames"]), 8)
            with self.assertRaises(FileExistsError), patch.object(shots, "commit", return_value="abc"):
                shots.capture(options)

    def test_missing_shot_leaves_incomplete_capture(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            exe = root / "game"
            exe.write_bytes(b"game")
            source = root / "suite.json"
            shots.write(source, self.spec())
            options = argparse.Namespace(sha="main", bin=str(exe), scenarios=str(source),
                root=root / "golden", label=None, repeats=1, game_dir=None, timeout=10)
            with patch.object(shots, "commit", return_value="abc"), patch.object(shots.subprocess, "run"), self.assertRaisesRegex(ValueError, "missing requested shot"):
                shots.capture(options)
            self.assertFalse(shots.read(options.root / "abc/capture.json")["complete"])

    def test_default_base_skips_incomplete(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for sha, complete in [("recent", False), ("older", True)]:
                (root / sha).mkdir()
                shots.write(root / sha / "capture.json", {"complete": complete})
            (root / "head").mkdir()
            shots.write(root / "head/capture.json", {"complete": True})
            with patch.object(shots, "run", return_value="head\nrecent\nolder\n"):
                self.assertEqual(shots.default_base(root, "head"), "older")
            with patch.object(shots, "run", return_value="uncaptured\n"), self.assertRaises(ValueError):
                shots.default_base(root, "head")

    def test_compare_clean_changed_missing_and_checks(self):
        for changed, missing, check_fail in [(False, False, False), (True, False, False),
                                              (False, True, False), (False, False, True)]:
            with self.subTest(changed=changed, missing=missing, check_fail=check_fail), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                spec = self.spec()
                for sha in ("a", "b"):
                    folder = root / sha / "walk"
                    folder.mkdir(parents=True)
                    frame = {"path": "tick.png", "key": "tick-000001-p00-r0", "tick": 1, "phase": 0}
                    (folder / "tick.png").write_bytes(b"png")
                    shots.write(folder / "frames.json", {"frames": [frame]})
                    shots.write(root / sha / "capture.json", {"complete": True, "sha": sha,
                        "scenarios": [{"name": "walk", "spec": spec, "frames": [] if missing and sha == "b" else [frame]}]})
                options = argparse.Namespace(root=root, refs=["a", "b"], out=root / "report",
                    img_check="img-check", img_diff="img-diff", ui=None, tol=3, max_pct=.1, mask=["hud"])

                def fake_tool(command):
                    if command[0] == "img-check":
                        return {"pass": not check_fail, "violations": ["black"] if check_fail else []}
                    self.assertIn("hud", command)
                    folder = Path(command[command.index("--out") + 1])
                    folder.mkdir()
                    (folder / "diff.png").write_bytes(b"png")
                    return {"pass": not changed}

                with patch.object(shots, "tool_verdict", side_effect=fake_tool), contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(shots.compare(options), int(changed or missing or check_fail))
                report = (options.out / "index.html").read_text(encoding="utf-8")
                self.assertIn("base, head, diff", report)
                self.assertEqual(shots.read(options.out / "verdict.json")["pass"], not (changed or missing or check_fail))

    def test_tool_errors_and_exit_contract(self):
        for code, stdout in [(2, ""), (0, '{"pass":false}'), (0, '[]'), (1, '{"pass":0}')]:
            with patch.object(shots.subprocess, "run", return_value=argparse.Namespace(returncode=code, stdout=stdout, stderr="bad")), self.assertRaises(ValueError):
                shots.tool_verdict(["fake"])

    def test_capture_ref_resolves_short_sha_and_keeps_labels(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "same-a").mkdir()
            shots.write(root / "same-a/capture.json", {})
            with patch.object(shots, "commit", return_value="fullsha") as resolver:
                self.assertEqual(shots.capture_ref(root, "same-a"), "same-a")
                resolver.assert_not_called()
                self.assertEqual(shots.capture_ref(root, "short"), "fullsha")


if __name__ == "__main__":
    unittest.main()
