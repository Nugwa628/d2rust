"""Binary cache tests: local/farm commands are faked, no compiler or SSH."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from tools.ci.bincache import Cache, QUEUE

SHA = "a" * 40


class CacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.calls = []
        self.cache = Cache(self.root / "bin", self.root / "repo", self.run_fake, env={})

    def run_fake(self, args, **kwargs):
        self.calls.append((args, kwargs))
        output = ""
        if args[:3] == ["git", "rev-parse", "--verify"] or args == ["git", "rev-parse", "HEAD"]:
            output = SHA
        if args[:3] == ["git", "worktree", "add"]:
            Path(args[-2]).mkdir(parents=True)
        if args[:2] == ["sh", QUEUE]:
            target = Path(args[args.index("--target-dir") + 1])
            profile = args[args.index("--profile") + 1]
            binary = target / profile / self.cache.binary(SHA, profile).name
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"fake executable")
        return subprocess.CompletedProcess(args, 0, output)

    def test_get_is_read_only_and_never_builds(self):
        with self.assertRaises(FileNotFoundError):
            self.cache.get(SHA)
        binary = self.cache.binary(SHA, "release")
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"cached")
        self.assertEqual(self.cache.get(SHA), binary)
        self.assertTrue(all(call[0][0] == "git" for call in self.calls))

    def test_local_build_checkout_queue_profile_and_cache_hit(self):
        for profile in ("release", "dist"):
            binary = self.cache.build(SHA, profile)
            self.assertEqual(binary.read_bytes(), b"fake executable")
        builds = [args for args, _ in self.calls if args[0] == "sh"]
        self.assertEqual(len(builds), 2)
        self.assertIn("12", builds[0])
        count = len(self.calls)
        self.cache.build(SHA)
        self.assertEqual(len(self.calls), count + 1)  # only resolve SHA
        self.assertFalse((self.cache.root / (SHA + ".lock")).exists())

    def test_failed_build_does_not_publish_and_unlocks(self):
        original = self.cache.runner
        def fail(args, **kwargs):
            if args[0] == "sh":
                raise subprocess.CalledProcessError(1, args)
            return original(args, **kwargs)
        self.cache.runner = fail
        with self.assertRaises(subprocess.CalledProcessError):
            self.cache.build(SHA)
        self.assertFalse(self.cache.binary(SHA, "release").exists())
        self.assertFalse((self.cache.root / (SHA + ".lock")).exists())

    def test_farm_absent_is_actionable_and_never_builds_locally(self):
        self.cache.env["D2_FARM_HOST"] = "fake-host"
        with self.assertRaisesRegex(RuntimeError, "merge Q5"):
            self.cache.build(SHA)
        self.assertFalse(any(args[0] == "sh" for args, _ in self.calls))

    def test_farm_command_and_download(self):
        farm = self.cache.repo / "tools/farm/farm.py"
        farm.parent.mkdir(parents=True)
        farm.touch()
        self.cache.env["D2_FARM_HOST"] = "fake-host"
        original = self.cache.runner
        def remote(args, **kwargs):
            if str(farm) in args:
                self.assertEqual(args[-2:], ["bin", SHA])
                binary = self.cache.binary(SHA, "release")
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"farm executable")
            return original(args, **kwargs)
        self.cache.runner = remote
        binary = self.cache.build(SHA)
        self.assertEqual(binary.name, "D2RUST-Game")
        self.assertEqual(binary.read_bytes(), b"farm executable")
        self.assertEqual(self.cache.get(SHA), binary)
        self.assertEqual(self.cache.env["D2_BIN_ROOT"], str(self.cache.root))
        self.assertFalse(any(args[0] == "sh" for args, _ in self.calls))

    def test_prune_keeps_newest_skips_busy_and_ignores_other_directories(self):
        self.cache.root.mkdir()
        for index, sha in enumerate(["a" * 40, "b" * 40, "c" * 40]):
            entry = self.cache.root / sha
            entry.mkdir()
            os.utime(entry, (index + 1, index + 1))
        (self.cache.root / "src").mkdir()
        (self.cache.root / ("b" * 40 + ".lock")).mkdir()
        self.assertEqual(self.cache.prune(1), ["a" * 40])
        self.assertTrue((self.cache.root / "src").is_dir())
        self.assertTrue((self.cache.root / ("c" * 40)).is_dir())

    def test_bad_sha_and_profiles_rejected_before_command(self):
        for sha in ["../escape", "--all", "a" * 41]:
            with self.assertRaises(ValueError):
                self.cache.build(sha)
        self.assertEqual(self.calls, [])
        with self.assertRaises(ValueError):
            self.cache.binary(SHA, "debug")
        with self.assertRaises(ValueError):
            self.cache.prune(-1)

    def test_modified_checkout_is_refused(self):
        original = self.cache.runner
        def dirty(args, **kwargs):
            if args == ["git", "status", "--porcelain"]:
                return subprocess.CompletedProcess(args, 0, " M Cargo.toml")
            return original(args, **kwargs)
        self.cache.runner = dirty
        with self.assertRaisesRegex(RuntimeError, "checkout is modified"):
            self.cache.build(SHA)
        self.assertFalse(any(args[0] == "sh" for args, _ in self.calls))

    def test_farm_must_actually_supply_a_binary(self):
        farm = self.cache.repo / "tools/farm/farm.py"
        farm.parent.mkdir(parents=True)
        farm.touch()
        self.cache.env["D2_FARM_HOST"] = "fake-host"
        with self.assertRaisesRegex(FileNotFoundError, "did not populate"):
            self.cache.build(SHA)

    def test_busy_entry_is_not_overwritten(self):
        self.cache.root.mkdir()
        (self.cache.root / (SHA + ".lock")).mkdir()
        with self.assertRaisesRegex(RuntimeError, "busy"):
            self.cache.build(SHA)


if __name__ == "__main__":
    unittest.main()
