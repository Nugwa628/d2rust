"""Affected-crate selection, including metadata and real git rename behavior."""
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import affected
import test_scope


class AffectedTests(unittest.TestCase):
    def setUp(self):
        self.workspace = (
            {"core": "crates/core", "client": "crates/client", "tool": "tools/tool"},
            {"core": set(), "client": {"core"}, "tool": {"client"}},
            {"core": {"tables/data.tsv"}},
            {"core": '', "client": '', "tool": ''}, {}, set())

    def test_transitive_dependents(self):
        self.assertEqual(affected.package_flags(["crates/core/src/lib.rs"], self.workspace),
                         "-p client -p core -p tool")

    def test_embedded_input(self):
        self.assertEqual(affected.package_flags(["tables/data.tsv"], self.workspace),
                         "-p client -p core -p tool")

    def test_workspace_and_docs(self):
        self.assertEqual(affected.package_flags(["Cargo.lock"], self.workspace), "--workspace")
        self.assertEqual(affected.package_flags(["docs/tools.md"], self.workspace), "")

    def test_directory_boundary(self):
        self.assertEqual(affected.package_flags(["crates/core-extra/x.rs"], self.workspace), "")

    def test_metadata_dependencies_include_dev_build_and_renames(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("core", "client"):
                (root / name).mkdir()
            meta = {"workspace_members": ["a", "b"], "packages": [
                {"id": "a", "name": "core", "manifest_path": str(root / "core/Cargo.toml"),
                 "features": {}, "targets": [], "dependencies": []},
                {"id": "b", "name": "client", "manifest_path": str(root / "client/Cargo.toml"),
                 "features": {}, "targets": [], "dependencies": [
                     {"name": "core", "rename": "alias", "kind": "dev"},
                     {"name": "core", "kind": "build"}]}]}
            import json
            with patch.object(test_scope.subprocess, "check_output", return_value=json.dumps(meta).encode()):
                workspace = test_scope.workspace(directory)
            self.assertEqual(affected.package_flags(["core/x.rs"], workspace), "-p client -p core")

    def test_git_commits_rename_local_and_untracked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, stderr=subprocess.DEVNULL)
            git("init", "-q")
            git("config", "user.name", "test")
            git("config", "user.email", "test@example.invalid")
            (root / "old.rs").write_text("old")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD").decode().strip()
            git("mv", "old.rs", "new.rs")
            git("commit", "-qm", "rename")
            (root / "new.rs").write_text("edited")
            (root / "extra.rs").write_text("new")
            self.assertEqual(affected.changed_files(base, root), ["extra.rs", "new.rs", "old.rs"])


if __name__ == "__main__":
    unittest.main()
