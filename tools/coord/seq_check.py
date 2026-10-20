"""Fail a branch that takes a sequence number main has not issued (METHODS.md M28).

Spec: specs/tools/scenario-diff.md §4 rule 3e.

Usage: seq_check.py [--base REF]   (default origin/main; run by pre_pr.sh and CI)
       seq_check.py --selftest

Against REF, for each file the branch changes (working tree included):
- tools/scenario-diff/orig_cache.py: the frozen format tables (FROZEN) equal
  REF's. An output change adds a tag file `tools/scenario-diff/formats/<script>/<tag>`.
- tools/trace-recorder/record_*.py: the TOOL version literal equals REF's (its
  tags come from the same tag files, tools/trace-recorder/tool_tags.py).
- specs/**.md: no rule revision `(rN` that REF's file lacks; a new revision
  is named `(r:<name>, <date>, <lane>)`, cited as `§n r:<name>`.
- any file: no `REC-<digits>` id that appears nowhere in REF; a new id is a
  name, `REC-<lane>` or `REC-<lane>-<name>`.
A file new on the branch has nothing to collide with and is not checked
(the REC rule still applies to it).
"""
import ast
import re
import subprocess
import sys

ORIG_CACHE = "tools/scenario-diff/orig_cache.py"
FROZEN = ("RECORDER_FORMAT", "OP_FORMAT", "OP_FORMAT_BY_SCRIPT", "OP_ALIASES_UNLESS",
          "RECORDER_ALIASES", "FORMAT_ALIASES", "RECORDER_ALIASES_UNLESS", "SLOT_OPS")
_RECORDER = re.compile(r"^tools/trace-recorder/record_\w+\.py$")
_REV = re.compile(r"\(r(\d+(?:\.\d+)*)[,;)]")
_REC = re.compile(r"\bREC-(\d+)\b")


def frozen_tables(src):
    """{name: value} of the module-level FROZEN assignments in `src`."""
    out = {}
    for node in ast.parse(src).body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 \
                and isinstance(node.targets[0], ast.Name) and node.targets[0].id in FROZEN:
            out[node.targets[0].id] = ast.literal_eval(node.value)
    return out


def frozen_changed(base_src, head_src):
    b, h = frozen_tables(base_src), frozen_tables(head_src)
    return sorted(n for n in FROZEN if b.get(n) != h.get(n))


def tool_version(src):
    """The TOOL literal: `TOOL = "v"`, `X, TOOL = "x", "v"` or `TOOL = tool_tags.tool(__file__, "v")`."""
    for node in ast.parse(src).body:
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        t, v = node.targets[0], node.value
        if isinstance(t, ast.Tuple) and isinstance(v, ast.Tuple):
            pairs = zip(t.elts, v.elts)
        else:
            pairs = [(t, v)]
        for name, val in pairs:
            if isinstance(name, ast.Name) and name.id == "TOOL":
                if isinstance(val, ast.Call) and len(val.args) == 2:
                    val = val.args[1]
                return val.value if isinstance(val, ast.Constant) else None
    return None


def new_revisions(base_text, head_text):
    return sorted(set(_REV.findall(head_text)) - set(_REV.findall(base_text)))


def new_recs(base_ids, head_text):
    return sorted(set(_REC.findall(head_text)) - base_ids, key=int)


def git(*args):
    return subprocess.run(("git",) + args, capture_output=True, text=True, check=True).stdout


def show(base, path):
    r = subprocess.run(("git", "show", f"{base}:{path}"), capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None


def check(base):
    try:
        git("rev-parse", "--verify", "-q", base + "^{commit}")
    except subprocess.CalledProcessError:
        print(f"seq_check: no {base} (git fetch origin main)", file=sys.stderr)
        return 2
    changed = [p for p in git("diff", "--name-only", base).splitlines() if p]
    # traces/orig-cache (4.8 GB, no REC ids) is left out: grepping it was 97 % of the time
    r = subprocess.run(("git", "grep", "-h", "-o", "-E", r"REC-[0-9]+", base, "--", ".", ":!traces/orig-cache"),
                       capture_output=True, text=True)   # exit 1: no match
    base_recs = set(_REC.findall(r.stdout))
    errs = []
    for path in changed:
        try:
            with open(path, encoding="utf-8") as f:
                head = f.read()
        except (OSError, UnicodeDecodeError):
            continue   # deleted or binary
        for n in new_recs(base_recs, head):
            errs.append(f"{path}: REC-{n} is not issued on {base}: name it (REC-<lane>[-<name>])")
        old = show(base, path)
        if old is None:
            continue
        if path == ORIG_CACHE:
            for n in frozen_changed(old, head):
                errs.append(f"{path}: frozen table {n} changed: add a tag file "
                            f"tools/scenario-diff/formats/<script>/<tag> instead (rule 3e)")
        elif _RECORDER.match(path) and tool_version(old) != tool_version(head):
            errs.append(f"{path}: TOOL {tool_version(old)!r} -> {tool_version(head)!r}: "
                        f"keep the number, add a format tag (rule 3e)")
        elif path.startswith("specs/") and path.endswith(".md"):
            for r in new_revisions(old, head):
                errs.append(f"{path}: revision (r{r} is not on {base}: name it (r:<name>, <date>, <lane>)")
    for e in errs:
        print("seq_check: " + e)
    print(f"seq_check: {len(changed)} changed files, {len(errs)} new sequence numbers")
    return 1 if errs else 0


def selftest():
    base = 'RECORDER_FORMAT = {"a.py": "a-1"}\nSLOT_OPS = {"p-1": "p"}\nX = 1\n'
    assert frozen_changed(base, base.replace("X = 1", "X = 2")) == []
    assert frozen_changed(base, base.replace("a-1", "a-2")) == ["RECORDER_FORMAT"]   # M08
    assert frozen_changed(base, base.replace('"a-1"}', '"a-1",  # note\n}')) == []
    assert frozen_changed(base, base.replace("SLOT_OPS", "SLOTS")) == ["SLOT_OPS"]
    assert tool_version('import x\nTOOL = "t r 0.1.0"  # c\n') == "t r 0.1.0"
    assert tool_version('TOOL = tool_tags.tool(__file__, "t r 0.1.0")\n') == "t r 0.1.0"
    assert tool_version('FORMAT, TOOL = "f-1", "t r 0.1.0"\n') == "t r 0.1.0"
    spec = "1. **A** (r1, 2026-10-01)\n2. **B** (r2; x)\nsee capture.md §4a r9\n"
    assert new_revisions(spec, spec + "3. **C** (r:lane-c, 2026-10-14, lane)\n") == []
    assert new_revisions(spec, spec + "3. **C** (r3, 2026-10-14)\n") == ["3"]
    assert new_revisions(spec, spec + "(r2.1)\n") == ["2.1"]
    assert new_revisions(spec, spec + "cites §4a r12 of capture.md\n") == []
    assert new_recs({"12", "300"}, "REC-12 REC-300 REC-ui-x REC-lane-a") == []
    assert new_recs({"12"}, "REC-12, REC-13 and REC-1200") == ["13", "1200"]
    print("seq_check selftest ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:2] == ["--selftest"]:
        raise SystemExit(selftest())
    a = sys.argv[1:]
    raise SystemExit(check(a[a.index("--base") + 1] if "--base" in a else "origin/main"))
