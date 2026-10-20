#!/usr/bin/env python3
"""New tools (METHODS M10): a tool added under tools/ comes with a selftest that runs.

Each `tools/**/*.py` the branch adds must (a) handle `--selftest` in its
source and (b) have its selftest called, as `tools/<path> --selftest` on a
non-comment line of .github/workflows/ci.yml or tools/coord/pre_pr.sh.
Exempt (no logic of their own to test): `__init__.py`, files under a
`tests/` directory.

  python3 tools/coord/new_tools.py [--base REF]   default origin/main (CI, pre_pr.sh)
  python3 tools/coord/new_tools.py --selftest

Added files: `git diff --name-only --diff-filter=A <merge-base> -- tools`
against the working tree (staged new files count). Where no merge-base is
reachable (CI fetches main with --depth=1 and checks out a shallow PR merge
commit) it falls back to `<REF>` itself. In CI the checkout is the PR merged
into main, so that diff is exactly the PR. Locally on an old branch the
fallback would report a tools file that main deleted after the branch point
as "added"; merging main clears it.
"""
import os
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
WIRING = (".github/workflows/ci.yml", "tools/coord/pre_pr.sh")


def exempt(path):
    return os.path.basename(path) == "__init__.py" or "/tests/" in "/" + path


def wired(path, wiring_text):
    """True if a non-comment line calls `<path> --selftest`."""
    call = path + " --selftest"
    return any(call in line and not line.lstrip().startswith("#")
               for line in wiring_text.splitlines())


def decide(added, source, wiring_text):
    """Errors for the added paths; source(path) -> text of that file."""
    errs = []
    for path in added:
        if not (path.startswith("tools/") and path.endswith(".py")) or exempt(path):
            continue
        if "--selftest" not in source(path):
            errs.append("%s: new tool has no `--selftest` (METHODS M10)" % path)
        if not wired(path, wiring_text):
            errs.append("%s: `%s --selftest` is not called in %s (METHODS M10)"
                        % (path, path, " or ".join(WIRING)))
    return errs


def git(*args):
    return subprocess.run(("git",) + args, cwd=REPO, capture_output=True, text=True)


def added_tools(base):
    mb = git("merge-base", base, "HEAD")
    since = mb.stdout.strip() if mb.returncode == 0 and mb.stdout.strip() else base
    r = git("diff", "--name-only", "--diff-filter=A", since, "--", "tools")
    if r.returncode != 0:
        raise SystemExit("new_tools: git diff failed: " + r.stderr.strip())
    return [p for p in r.stdout.splitlines() if p], since


def read(path):
    try:
        with open(os.path.join(REPO, path), encoding="utf-8") as fh:
            return fh.read()
    except (OSError, UnicodeDecodeError):
        return ""


def check(base):
    if git("rev-parse", "--verify", "-q", base + "^{commit}").returncode != 0:
        print("new_tools: no %s (git fetch origin main)" % base, file=sys.stderr)
        return 2
    added, since = added_tools(base)
    errs = decide(added, read, "\n".join(read(w) for w in WIRING))
    for e in errs:
        print("new_tools: " + e)
    n = sum(1 for p in added if p.endswith(".py") and not exempt(p))
    print("new_tools: %d new tool(s) since %s, %d error(s)" % (n, since[:12], len(errs)))
    return 1 if errs else 0


def selftest():
    good = 'if sys.argv[1:] == ["--selftest"]:\n    selftest()\n'
    src = {"tools/a.py": good, "tools/coord/b.py": good, "tools/c.py": "print(1)\n",
           "tools/x/__init__.py": "", "tools/x/tests/t.py": "", "tools/d.sh": "",
           "crates/e.py": ""}
    wiring = ("      - run: python3 tools/a.py --selftest\n"
              "python3 tools/coord/b.py --selftest  # M10\n"
              "# python3 tools/c.py --selftest\n")
    fails = []

    def expect(name, added, n, want=""):
        errs = decide(added, src.get, wiring)
        if len(errs) != n or (want and not all(want in e for e in errs)):
            fails.append("%s: got %r" % (name, errs))

    expect("wired tools with a selftest pass", ["tools/a.py", "tools/coord/b.py"], 0)
    expect("exempt and non-tool files are skipped",
           ["tools/x/__init__.py", "tools/x/tests/t.py", "tools/d.sh", "crates/e.py"], 0)
    # M08: each perturbation is reported
    src["tools/a.py"] = "print(1)\n"
    expect("no --selftest in source", ["tools/a.py"], 1, "no `--selftest`")
    src["tools/a.py"] = good
    expect("commented-out call is not wiring, no selftest either", ["tools/c.py"], 2, "tools/c.py")
    src["tools/c.py"] = good
    expect("commented-out call is not wiring", ["tools/c.py"], 1, "not called")
    src["tools/f.py"] = good
    expect("unwired new tool", ["tools/f.py"], 1, "tools/f.py --selftest` is not called")
    src["tools/coord/a.py"] = good
    expect("a call to another path does not count", ["tools/coord/a.py"], 1, "not called")
    if wired("tools/a.py", "python3 tools/a.py --check\n"):
        fails.append("a non-selftest call counted as wiring")
    for f in fails:
        print("FAIL " + f)
    print("new_tools selftest: " + ("ok" if not fails else "%d failed" % len(fails)))
    return 1 if fails else 0


def main(argv):
    if argv == ["--selftest"]:
        return selftest()
    if argv and (len(argv) != 2 or argv[0] != "--base"):
        print(__doc__)
        return 2
    return check(argv[1] if argv else "origin/main")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
