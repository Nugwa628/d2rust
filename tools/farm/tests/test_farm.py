"""No network: exercise the production SSH boundary through a fake runner."""
import contextlib
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import farm

SHA = "a" * 40


class FakeRunner:
    def __init__(self, code=0):
        self.calls = []
        self.code = code

    def run(self, argv, **kwargs):
        self.calls.append((argv, kwargs))
        print("fake remote: compiling\nfake remote: completed", flush=True)
        if argv[0] == "scp" and not self.code:
            Path(argv[-1]).write_bytes(b"synthetic executable")
        return self.code


class FarmTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.config = farm.Config(host="farm@example", key="key with spaces",
                                  root="/srv/farm space", cache="/cache",
                                  bins=self.root / "bin", candidates=self.root / "candidates",
                                  summary=self.root / "summary.md")

    def call(self, args, runner=None):
        runner = runner or FakeRunner()
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            rc = farm.main(args, config=self.config, runner=runner)
        return rc, output.getvalue(), runner

    def test_gates_fetch_from_forge_and_stream_logs_and_exit_code(self):
        for kind in ("build", "clippy", "test"):
            with self.subTest(kind=kind):
                rc, log, runner = self.call([kind, SHA, "-p", "d2-client"], FakeRunner(17))
                self.assertEqual(rc, 17)
                self.assertIn("fake remote: compiling\nfake remote: completed", log)
                argv, kw = runner.calls[0]
                self.assertEqual(argv[0], "ssh")
                self.assertIn("BatchMode=yes", argv)
                self.assertIn("StrictHostKeyChecking=yes", argv)
                self.assertIn("key with spaces", argv)
                script = kw["script"]
                self.assertIn("set -euo pipefail", script)
                self.assertIn("'+refs/heads/*:refs/remotes/origin/*'", script)
                self.assertIn(farm.FORGE, script)
                self.assertIn("worktree add --detach", script)
                self.assertIn("flock 9", script)
                self.assertIn("root='/srv/farm space'", script)
                self.assertIn(shlex.join(farm.gate_command(kind, ["d2-client"])), script)

    def test_bad_sha_host_and_package_never_reach_ssh(self):
        for args in (["build", "HEAD;evil"], ["clippy", SHA, "-p", "foo;evil"]):
            rc, _, runner = self.call(args)
            self.assertEqual(rc, 2)
            self.assertFalse(runner.calls)
        self.config.host = "-oProxyCommand=evil"
        rc, _, runner = self.call(["status"])
        self.assertEqual(rc, 2)
        self.assertFalse(runner.calls)

    def test_environment_defaults_are_absolute_on_the_current_platform(self):
        with patch.dict(os.environ):
            for key in list(os.environ):
                if key.startswith("D2_FARM_") or key == "D2_BIN_ROOT":
                    del os.environ[key]
            config = farm.Config.env()
        self.assertTrue(config.bins.is_absolute())
        self.assertTrue(config.candidates.is_absolute())
        self.assertTrue(config.summary.is_absolute())
        self.assertEqual(config.host, "")
        config.validate()

    def test_binary_download_publishes_only_success_and_linux_metadata(self):
        rc, _, _ = self.call(["bin", SHA])
        self.assertEqual(rc, 0)
        dest = self.config.bins / SHA / "D2RUST-Game"
        self.assertEqual(dest.read_bytes(), b"synthetic executable")
        self.assertEqual(json.loads(dest.with_name("farm.json").read_text())["platform"], "linux")
        rc, _, _ = self.call(["bin", SHA], FakeRunner(9))
        self.assertEqual(rc, 9)
        self.assertEqual(dest.read_bytes(), b"synthetic executable")
        self.assertFalse(dest.with_name(dest.name + ".part").exists())

    def candidate(self, sig):
        folder = self.config.candidates / farm.signature_id(sig)
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "candidate.json").write_text(json.dumps({"sig": sig, "status": "Unverified"}))
        (folder / "failure.scn").write_text("synthetic scenario")

    def test_candidate_mirror_and_summary_are_idempotent_by_signature(self):
        self.candidate("desync:player-position")
        self.candidate("panic:some failure")
        rc, _, runner = self.call(["candidates", "pull"])
        self.assertEqual(rc, 0)
        argv, _ = runner.calls[0]
        self.assertEqual(argv[0], "rsync")
        self.assertIn("--safe-links", argv)
        self.assertIn("--exclude=*", argv)
        first = self.config.summary.read_text(encoding="utf-8")
        self.assertEqual(first.count("**Unverified**"), 2)
        self.call(["candidates", "pull"])
        self.assertEqual(first, self.config.summary.read_text(encoding="utf-8"))

    def test_mirror_failure_does_not_append_summary(self):
        self.candidate("guid:reuse")
        rc, _, _ = self.call(["candidates", "pull"], FakeRunner(23))
        self.assertEqual(rc, 23)
        self.assertFalse(self.config.summary.exists())

    def test_local_commands_announce_and_do_not_use_ssh(self):
        self.config.host = ""
        for args in (["status"], ["candidates", "pull"], ["bin", SHA],
                     ["build", SHA], ["clippy", SHA], ["test", SHA],
                     ["soak", "--hours", "nan", "--seeds", "1"]):
            with self.subTest(args=args):
                rc, log, runner = self.call(args)
                self.assertIn("local (D2_FARM_HOST unset)", log)
                self.assertFalse(runner.calls)
                self.assertIn(rc, (0, 2))

    def test_remote_soak_has_lock_time_seed_and_local_recursion_guard(self):
        rc, _, runner = self.call(["soak", "--hours", "8", "--seeds", "1,7,42"])
        self.assertEqual(rc, 0)
        script = runner.calls[0][1]["script"]
        self.assertIn("flock -n 9", script)
        self.assertIn("--local --hours 8.0 --seeds 1,7,42", script)
        self.assertIn("D2_GAME_DIR=/cache/game", script)
        self.assertIn("D2_FARM_CANDIDATES=/cache/candidates", script)

    def test_collect_groups_signatures_and_preserves_start_save(self):
        soak = farm.module(farm.REPO / "tools/soak/soak.py", "test_soak")
        source = self.root / "source"
        source.mkdir()
        log = source / "reduced.log"
        save = source / "start.d2s"
        save.write_bytes(b"synthetic save")
        log.write_text(f"soak-log 1 tick_rate=60 save={save.as_posix()} kit=0\n3 roundtrip\n")
        records = [{"sig": "roundtrip:stats", "step": 9, "reduced": str(log)}] * 2
        (source / "findings.jsonl").write_text("".join(json.dumps(r) + "\n" for r in records))
        self.assertEqual(farm.collect(soak, source, self.config.candidates), 1)
        self.assertEqual(farm.collect(soak, source, self.config.candidates), 0)
        folder = self.config.candidates / farm.signature_id("roundtrip:stats")
        scenario = (folder / "failure.scn").read_text()
        envelope = json.loads(scenario.split("\n", 1)[1])
        self.assertIn("3 roundtrip", envelope["log"])
        self.assertNotIn(str(source), envelope["log"])
        self.assertEqual(envelope["steps"], 409)
        self.assertEqual(len(list(folder.glob("*.d2s"))), 1)
        self.assertEqual(json.loads((folder / "candidate.json").read_text())["status"], "Unverified")

    def test_export_validation_needs_no_game_or_binaries(self):
        soak = farm.module(farm.REPO / "tools/soak/soak.py", "test_export")
        log = self.root / "reduced.log"
        log.write_text("soak-log 1 tick_rate=60 new=amazon kit=0\n3 move -1 42\n")
        output = self.root / "export.scn"
        soak.export_scenario(log, "model:missing", output, 4)
        payload = json.loads(output.read_text().split("\n", 1)[1])
        self.assertIn("3 move -1 42", payload["log"])
        with self.assertRaises(ValueError):
            soak.export_scenario(log, "model:missing", output, 3)

    def test_runner_streams_child_output_and_preserves_failure(self):
        # In-process/loopback only: the real runner executes Python, never SSH.
        self.assertEqual(farm.Runner().run([os.sys.executable, "-c", "print('runner live log'); raise SystemExit(7)"]), 7)

    def test_clean_local_test_uses_ci_wait_then_queue_and_clears_farm_host(self):
        self.config.host = ""
        with patch.object(farm.subprocess, "check_output", side_effect=[SHA, ""]), \
                patch.object(Path, "is_file", return_value=True):
            rc, _, runner = self.call(["test", SHA, "-p", "d2-client"])
        self.assertEqual(rc, 0)
        self.assertEqual(len(runner.calls), 2 if os.name == "nt" else 1)
        if os.name == "nt":
            self.assertIn("ci-wait.sh", runner.calls[0][0][1])
            self.assertIn("build-slot.sh", runner.calls[1][0][1])
        self.assertEqual(runner.calls[-1][1]["env"]["D2_FARM_HOST"], "")

    def test_local_soak_missing_binary_reports_cannot_run(self):
        self.config.host = ""
        with patch.object(Path, "is_file", return_value=False):
            rc, log, runner = self.call(["soak", "--hours", "1", "--seeds", "7"])
        self.assertEqual(rc, 2)
        self.assertIn("local soak unavailable", log)
        self.assertFalse(runner.calls)

    def test_real_runner_stdin_script_preserves_exit_code(self):
        self.assertEqual(farm.Runner().run([os.sys.executable, "-c",
                         "import sys; exec(sys.stdin.read())"], script="raise SystemExit(19)"), 19)

    def test_generated_remote_scripts_parse_without_executing(self):
        shell = shutil.which("bash") if os.name != "nt" else "C:/Program Files/Git/bin/bash.exe"
        if not shell or not Path(shell).is_file():
            self.skipTest("bash unavailable for syntax check")
        for args in (["build", SHA, "-p", "d2-client"],
                     ["soak", "--hours", "1", "--seeds", "1,7"]):
            rc, _, runner = self.call(args)
            self.assertEqual(rc, 0)
            script = runner.calls[0][1]["script"]
            parsed = subprocess.run([shell, "-n"], input=script, text=True, capture_output=True)
            self.assertEqual(parsed.returncode, 0, parsed.stderr)

    def test_soak_publishes_during_each_seed_slice_without_a_real_game(self):
        soak = farm.module(farm.REPO / "tools/soak/soak.py", "campaign_stub")
        tool = self.root / "synthetic-tool"
        tool.touch()
        soak.CLIENT = soak.D2S = str(tool)
        log = self.root / "reduced.log"
        log.write_text("soak-log 1 tick_rate=60 new=amazon kit=0\n0 wait 1\n")
        calls = []

        def campaign(args, on_finding):
            calls.append(args.first_seed)
            entry = {"sig": "test:failure", "reduced": str(log), "step": 0}
            (Path(args.out) / "findings.jsonl").write_text(json.dumps(entry) + "\n")
            on_finding(entry)
            self.assertTrue((self.config.candidates / farm.signature_id(entry["sig"]) / "failure.scn").is_file())

        soak.campaign = campaign
        with patch.object(farm, "module", return_value=soak), \
                patch.object(farm.time, "monotonic", side_effect=[0, 1, 2]), \
                patch.dict(os.environ):
            self.assertEqual(farm.soak_local(self.config, 1, [7, 42], self.root / "soak"), 0)
        self.assertEqual(calls, [7, 42])


if __name__ == "__main__":
    unittest.main()
