#!/usr/bin/env python3
"""Print nextest package flags affected by BASE...HEAD and local edits.

Uses the CI scope engine, including reverse normal/dev/build dependencies,
embedded inputs and workspace-wide build inputs. Empty output means no tests.
The merge train continues to run the whole workspace.
"""
import argparse
import subprocess
from pathlib import Path

import test_scope


def changed_files(base, root):
    """Include both sides of renames, committed changes, and local edits."""
    commands = [
        ["git", "diff", "--name-only", "--no-renames", "-z", f"{base}...HEAD"],
        ["git", "diff", "--name-only", "--no-renames", "-z", "HEAD"],
        ["git", "ls-files", "--others", "--exclude-standard", "-z"],
    ]
    paths = set()
    for command in commands:
        paths.update(p for p in subprocess.check_output(command, cwd=root)
                     .decode("utf-8").split("\0") if p)
    return sorted(paths)


def package_flags(changed, workspace):
    members, deps, includes, names, features, _ = workspace
    selected, _ = test_scope.plan(changed, members, deps, includes, names)
    return test_scope.args(selected, [], features)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", metavar="BASE")
    options = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    print(package_flags(changed_files(options.base, root), test_scope.workspace(str(root))))


if __name__ == "__main__":
    main()
