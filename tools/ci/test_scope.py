#!/usr/bin/env python3
"""Test scope: which workspace packages a PR's tests must cover.

ci.yml's check job runs fmt and clippy on the whole workspace, but on a pull
request it runs the tests of only the packages the change can affect, plus
every package that depends on them. Pushes to main test the whole workspace.

A package is affected when the PR changes
  - a file under its directory (the innermost package wins), or
  - a file it compiles in (`include_str!` / `include_bytes!` naming the path), or
  - a file outside every package that its sources name: the file's path or
    its directory ("tables/ui/x.tsv", "tables/ui"), or its top directory as
    a whole string ("tables", for `dir.join("tables")`), or
  - a package it depends on (normal, dev or build dependency), transitively.
The whole workspace is tested when the change touches the workspace's build
inputs (root Cargo.toml / Cargo.lock, rust-toolchain.toml, .cargo/, .config/,
ci.yml, this script). Any other file outside every package that no package
names (docs, scripts, other workflows) affects no tests; push to main still
tests everything.

  python3 tools/ci/test_scope.py (--base <commit> | --all) [--doc] [--feature PKG/FEAT ...]
      prints the `cargo nextest run` package args: `--workspace`,
      `-p a -p b`, or nothing (no tests to run). Each --feature is appended
      as `--features PKG/FEAT` when PKG is in scope and declares FEAT.
      --doc: the `cargo test --doc` args instead (packages with a library
      only). The reasons go to stderr for the job log.
      TEST_SCOPE_METADATA=<file> reads `cargo metadata --no-deps` output from
      a file instead of running cargo (local tests).
  python3 tools/ci/test_scope.py --selftest

Adapted from D2RUST-MAIN's tools/ci/gate_plan.py.
"""
import argparse
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# Build inputs of every package: a change here tests the whole workspace.
WHOLE = re.compile(
    r"^(Cargo\.toml|Cargo\.lock|rust-toolchain(\.toml)?|\.cargo/|\.config/"
    r"|\.github/workflows/ci\.yml$|tools/ci/test_scope\.py$)")
# Outside every package and read by no test.
IGNORED = re.compile(
    r"^(docs/|\.claude/|\.github/|fuzz/|[^/]+\.md$|LICENSE$|\.gitignore$|\.gitattributes$)")
INCLUDE = re.compile(r"include_(?:str|bytes)!\(\s*(?:concat!\([^)]*?)?\"([^\"]+)\"")


def owner(path, members):
    """The innermost package whose directory holds `path`, or None."""
    return max((n for n, d in members.items() if path.startswith(d + "/")),
               key=lambda n: len(members[n]), default=None)


def plan(changed, members, deps, includes, names):
    """changed: repo paths; members: {name: dir}; deps: {name: set(names it depends on)};
    includes: {name: set(repo paths it compiles in)}; names: {name: its sources' text}. Returns (None = whole workspace, or a sorted list, reasons)."""
    why = []
    hit = set()
    for p in changed:
        if WHOLE.match(p):
            return None, ["%s: workspace build input" % p]
        o = owner(p, members)
        if o:
            hit.add(o)
            why.append("%s: in %s" % (p, o))
            continue
        inc = {n for n, i in includes.items() if p in i}
        if inc:
            hit |= inc
            why.append("%s: compiled into %s" % (p, " ".join(sorted(inc))))
            continue
        if IGNORED.match(p):
            continue
        readers = {n for n, t in names.items() if named(p, t)}
        if readers:
            hit |= readers
            why.append("%s: named by %s" % (p, " ".join(sorted(readers))))
        else:
            why.append("%s: outside every package, named by none" % p)
    rdeps = {}
    for n, ds in deps.items():
        for d in ds:
            rdeps.setdefault(d, set()).add(n)
    todo, out = list(hit), set(hit)
    while todo:
        for r in rdeps.get(todo.pop(), ()):
            if r not in out:
                out.add(r)
                todo.append(r)
    return sorted(out), why


def named(path, text):
    """Whether source `text` names repo `path`, its directory, or (as a whole
    string) its top directory."""
    parts = path.split("/")
    if len(parts) < 2:
        return False
    if '"%s"' % parts[0] in text:
        return True
    return any("/".join(parts[:i]) in text for i in range(len(parts), 1, -1))


def args(got, features, pkg_features, libs=None):
    """`cargo nextest run` args for a plan() result; features: ["pkg/feat", ...];
    libs: the packages with a library target (doc tests), or None (all)."""
    if got is not None and libs is not None:
        got = [n for n in got if n in libs]
    if got is not None and not got:
        return ""
    out = ["--workspace"] if got is None else ["-p " + n for n in got]
    for f in features:
        pkg, _, feat = f.partition("/")
        if (got is None or pkg in got) and feat in pkg_features.get(pkg, ()):
            out.append("--features " + f)
    return " ".join(out)


def workspace(root=ROOT):
    if os.environ.get("TEST_SCOPE_METADATA"):
        with open(os.environ["TEST_SCOPE_METADATA"], encoding="utf-8") as fh:
            meta = json.load(fh)
    else:
        meta = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"], cwd=root))
    pkgs = [p for p in meta["packages"] if p["id"] in meta["workspace_members"]]
    members, deps, includes, names, features, libs = {}, {}, {}, {}, {}, set()
    all_names = {p["name"] for p in pkgs}
    for p in pkgs:
        d = os.path.relpath(os.path.dirname(p["manifest_path"]), root).replace(os.sep, "/")
        members[p["name"]] = d
        features[p["name"]] = set(p["features"])
        if any("lib" in t["kind"] for t in p["targets"]):
            libs.add(p["name"])
        deps[p["name"]] = {x["name"] for x in p["dependencies"] if x["name"] in all_names}
        inc, text = set(), []
        for base, dirs, files in os.walk(os.path.join(root, d)):
            dirs[:] = [x for x in dirs if x != "target"]
            for f in files:
                if not f.endswith(".rs"):
                    continue
                src_dir = os.path.relpath(base, root)
                with open(os.path.join(base, f), encoding="utf-8", errors="replace") as fh:
                    src = fh.read()
                for rel in INCLUDE.findall(src):
                    inc.add(os.path.normpath(os.path.join(src_dir, rel)).replace(os.sep, "/"))
                text.append(src)
        includes[p["name"]] = inc
        names[p["name"]] = "\n".join(text)
    return members, deps, includes, names, features, libs


def selftest():
    members = {"a": "crates/a", "b": "crates/b", "c": "crates/c", "t": "tools/t", "ab": "crates/a/b"}
    deps = {"a": set(), "b": {"a"}, "c": {"b"}, "t": set(), "ab": set()}
    inc = {"a": set(), "b": {"specs/x.md"}, "c": set(), "t": set(), "ab": set()}
    nm = {"a": 'let p = root.join("../../tables/ui/keys.tsv");', "b": 'dir.join("facts")', "c": "", "t": "", "ab": ""}

    def p(*changed):
        return plan(list(changed), members, deps, inc, nm)[0]
    assert p("docs/notes.md", "README.md", ".claude/x", ".github/workflows/other.yml") == []
    assert p("crates/c/src/lib.rs") == ["c"]
    assert p("crates/a/src/lib.rs") == ["a", "b", "c"]          # reverse deps, transitively
    assert p("crates/a/b/src/lib.rs") == ["ab"]                  # innermost package wins
    assert p("specs/x.md") == ["b", "c"]                         # compiled-in file
    assert p("tables/ui/keys.tsv") == ["a", "b", "c"]            # file named by a
    assert p("tables/ui/other.tsv") == ["a", "b", "c"]           # its directory named by a
    assert p("tables/render/x.tsv") == []                        # named by none
    assert p("facts/x/y.json") == ["b", "c"]                     # "facts" joined by b
    assert p("tools/coord/x.py", "tables") == []                 # scripts; a root file
    assert p("Cargo.lock") is None and p("crates/c/x.rs", "rust-toolchain.toml") is None
    assert p(".github/workflows/ci.yml") is None and p("tools/ci/test_scope.py") is None
    assert p("crates/a/Cargo.toml") == ["a", "b", "c"]           # a member manifest is the member's
    assert p("tools/t/main.rs", "LICENSE") == ["t"]
    feats = {"c": {"dylib"}, "a": set()}
    assert args(None, ["c/dylib"], feats) == "--workspace --features c/dylib"
    assert args(["a", "c"], ["c/dylib"], feats) == "-p a -p c --features c/dylib"
    assert args(["a"], ["c/dylib"], feats) == "-p a"             # c not in scope
    assert args(["a", "c"], ["a/dylib"], feats) == "-p a -p c"   # feature not declared (yet)
    assert args([], ["c/dylib"], feats) == ""
    assert args(["a", "t"], [], feats, libs={"a"}) == "-p a"   # doc tests: libraries only
    assert args(["t"], [], feats, libs={"a"}) == ""
    assert args(None, [], feats, libs={"a"}) == "--workspace"
    m = INCLUDE.findall('const S: &str = include_str!("../../../specs/sim/tick.md"); include_bytes!(concat!(env!("X"), "/f.bin"))')
    assert m[0] == "../../../specs/sim/tick.md", m
    print("test_scope selftest: ok")


def main(argv):
    ap = argparse.ArgumentParser(description="See the module docstring.")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--base", help="merge base of the pull request")
    g.add_argument("--all", action="store_true", help="the whole workspace")
    g.add_argument("--selftest", action="store_true")
    ap.add_argument("--doc", action="store_true", help="cargo test --doc args")
    ap.add_argument("--feature", action="append", default=[], metavar="PKG/FEAT")
    a = ap.parse_args(argv)
    if a.selftest:
        selftest()
        return 0
    ws = workspace()
    if a.all:
        got, why = None, ["--all"]
    else:
        # Two-dot diff against the merge base; no rename detection, so a
        # move names both paths and only trees are compared (the base may
        # be a blob-less shallow fetch).
        changed = subprocess.check_output(
            ["git", "diff", "--no-renames", "--name-only", a.base, "HEAD"], cwd=ROOT, text=True).split()
        got, why = plan(changed, *ws[:4])
    for w in why:
        print("test scope: " + w, file=sys.stderr)
    print("test scope: %s" % ("whole workspace" if got is None else " ".join(got) or "no tests"), file=sys.stderr)
    print(args(got, a.feature, ws[4], ws[5] if a.doc else None))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
