#!/usr/bin/env python3
"""Local-only offscreen picture suite. See docs/tools/shots.md.

No builds, window automation, uploads, or Python imaging dependencies. Game
binaries come from Q1's cache or --bin; PNG rules and diffs use img-diff tools.
"""
import argparse
import hashlib
import html
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
GOLDEN = Path("E:/work/artifacts/golden" if os.name == "nt" else "/cache/golden")


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, capture_output=True,
                          text=True, **kwargs).stdout.strip()


def commit(value):
    return run(["git", "rev-parse", "--verify", f"{value}^{{commit}}"], cwd=ROOT)


def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def binary(sha, explicit):
    if explicit:
        path = Path(explicit).resolve()
        if not path.is_file():
            raise ValueError(f"binary not found: {path}")
        return path
    cache = ROOT / "tools/ci/bincache.py"
    if not cache.is_file():
        raise ValueError("bincache.py is not available; supply --bin")
    output = run([sys.executable, cache, "get", sha])
    # The cache's stdout is a path; diagnostics belong on stderr.
    path = Path(output).resolve()
    if not path.is_file():
        raise ValueError(f"bincache get did not return a binary path: {output}")
    return path


def safe_name(value):
    if not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]*", value):
        raise ValueError(f"invalid scenario name: {value!r}")
    return value


def validate_scenario(s):
    safe_name(s["name"])
    rate = s.get("tick_rate", 25)
    if type(rate) is not int or not 1 <= rate <= 1000:
        raise ValueError("tick_rate must be an integer in 1..=1000")
    args = s.get("args", [])
    if not isinstance(args, list) or not all(isinstance(a, str) for a in args):
        raise ValueError("scenario args must be a list of strings")
    # Only input/setup/display flags: capture owns the lifecycle and hidden
    # window flags, so a manifest cannot accidentally launch visible play.
    arity = {"--new": 2, "--save": 1, "--seed": 1, "--difficulty": 1,
             "--input": 1, "--poke": 1, "--send": 1, "--res": 1}
    i = 0
    while i < len(args):
        option = args[i]
        if option not in arity or i + arity[option] >= len(args):
            raise ValueError(f"unsupported/incomplete play option: {option}")
        i += arity[option] + 1
    if ("--new" in args) == ("--save" in args):
        raise ValueError("scenario needs exactly one --new or --save")
    ticks = s.get("ticks", [60, 120])
    if not ticks or any(type(t) is not int or t <= 0 for t in ticks) or ticks != sorted(set(ticks)):
        raise ValueError("ticks must be sorted unique positive integers")
    phases = s.get("phases", [0, .25, .5, .75])
    if (not phases or phases[0] != 0 or
            any(type(p) not in (int, float) or not 0 <= p < 1 for p in phases)):
        raise ValueError("phases must start at zero and stay below one")
    ms = [int(p * 1000 / rate + .5) for p in phases]
    if ms != sorted(set(ms)):
        raise ValueError("phases must increase by at least 1 ms")
    if not re.fullmatch(r"[1-9][0-9]*x[1-9][0-9]*", s.get("size", "1280x720")):
        raise ValueError("size must be WIDTHxHEIGHT")
    return args, ticks, phases, ms


def scenarios(source):
    path = Path(source).resolve()
    files = sorted(path.glob("*.json")) if path.is_dir() else [path]
    result = []
    for file in files:
        data = read(file)
        entries = data if isinstance(data, list) else [data]
        for s in entries:
            validate_scenario(s)
            # Save paths are relative to the manifest, like scenario scripts.
            args = list(s["args"])
            if "--save" in args:
                i = args.index("--save") + 1
                args[i] = str((file.parent / args[i]).resolve())
            result.append(dict(s, args=args))
    names = [s["name"] for s in result]
    if not names or len(names) != len(set(names)):
        raise ValueError("need at least one scenario and unique names")
    return result


def capture(options):
    sha = commit(options.sha)
    exe = binary(sha, options.bin)
    specs = scenarios(options.scenarios)
    dest = options.root.resolve() / (safe_name(options.label) if options.label else sha)
    # Never overwrite evidence or mix a partial run with a prior capture.
    dest.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    with exe.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    manifest = {"format": "shots 1", "sha": sha, "complete": False,
                "binary": str(exe), "binary_sha256": digest,
                "scenarios": []}
    write(dest / "capture.json", manifest)
    for spec in specs:
        args, ticks, phases, milliseconds = validate_scenario(spec)
        folder = dest / spec["name"]
        folder.mkdir()
        frames = []
        # T5 selects one phase per process. Repeat each phase to check that
        # nothing changed at the same tick/phase, then inspect phases together.
        for repeat in range(options.repeats):
            for index, phase in enumerate(phases):
                shot_dir = folder / f"r{repeat}-p{index}"
                shot_dir.mkdir()
                env = dict(os.environ, D2RUST_HOME=str(shot_dir / "user"),
                           D2RUST_LOG_DIR=str(shot_dir / "logs"))
                (shot_dir / "user/saves").mkdir(parents=True)
                command = [exe, "play", *args, "--offscreen", "--no-sound", "--mute",
                           "--size", spec.get("size", "1280x720"),
                           "--display-clock", "phases:" + ",".join(map(str, phases)),
                           "--shot-phase", str(phase), "--shot-at", ",".join(map(str, ticks)),
                           "--shot-dir", shot_dir]
                if options.game_dir:
                    command += ["--game-dir", options.game_dir]
                with (shot_dir / "capture.log").open("w", encoding="utf-8") as log:
                    subprocess.run(list(map(str, command)), cwd=ROOT, env=env, stdout=log,
                                   stderr=subprocess.STDOUT, timeout=options.timeout, check=True)
                # Verify exact requested shots, not merely process success.
                for tick in ticks:
                    name = f"tick-{tick:06}" + (f"-f{index}" if index else "") + ".png"
                    path = shot_dir / name
                    if not path.is_file():
                        raise ValueError(f"missing requested shot: {path}; see {shot_dir / 'capture.log'}")
                    frames.append({"path": path.relative_to(folder).as_posix(), "tick": tick,
                                   "phase": milliseconds[index], "repeat": repeat,
                                   "key": f"tick-{tick:06}-p{milliseconds[index]:02}-r{repeat}"})
        write(folder / "frames.json", {"frames": frames})
        manifest["scenarios"].append({"name": spec["name"], "spec": spec, "frames": frames})
        write(dest / "capture.json", manifest)
    manifest.update(complete=True, wall_seconds=round(time.monotonic() - started, 3))
    write(dest / "capture.json", manifest)
    print(dest)
    return 0


def default_base(root, head):
    """Newest complete captured ancestor in date order, excluding HEAD."""
    for sha in run(["git", "rev-list", "--date-order", head], cwd=ROOT).splitlines():
        if sha == head:
            continue
        path = root / sha / "capture.json"
        if path.is_file() and read(path).get("complete"):
            return sha
    raise ValueError("no complete ancestor golden capture; pass BASE explicitly")


def tool_verdict(command):
    p = subprocess.run(list(map(str, command)), capture_output=True, text=True)
    if p.returncode not in (0, 1):
        raise ValueError(f"tool failed ({p.returncode}): {command}\n{p.stderr}")
    value = json.loads(p.stdout)
    if not isinstance(value, dict) or type(value.get("pass")) is not bool or value["pass"] != (p.returncode == 0):
        raise ValueError("tool exit code disagrees with verdict")
    return value


def capture_ref(root, value):
    try:
        label = safe_name(value)
    except ValueError:
        label = None
    if label and (root / label / "capture.json").is_file():
        return label
    return commit(value)


def report_image(path, report_dir):
    if not path.is_file():
        return "<p>Missing image</p>"
    url = Path(os.path.relpath(path, report_dir)).as_posix()
    from urllib.parse import quote
    return f'<a href="{html.escape(quote(url))}"><img loading="lazy" src="{html.escape(quote(url))}"></a>'


def compare(options):
    root = options.root.resolve()
    if len(options.refs) > 2:
        raise ValueError("compare takes [BASE] HEAD")
    head = capture_ref(root, options.refs[-1])
    base = capture_ref(root, options.refs[0]) if len(options.refs) == 2 else default_base(root, commit(head))
    labels = [safe_name(s) for s in (base, head)]
    # Named labels support same-build repeat experiments without overwriting a sha.
    captures = [root / label for label in labels]
    a, b = [read(c / "capture.json") for c in captures]
    if not all(m.get("complete") for m in (a, b)):
        raise ValueError("cannot compare incomplete captures")
    output = options.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    by_name = [{s["name"]: s for s in m["scenarios"]} for m in (a, b)]
    results, sections = [], []
    for name in sorted(set(by_name[0]) | set(by_name[1])):
        safe_name(name)
        folder = output / name
        folder.mkdir()
        pair = [m.get(name) for m in by_name]
        violations, rows = [], []
        check = {"pass": False, "violations": [{"rule": "missing-scenario"}]}
        if pair[1]:
            check_cmd = [options.img_check, captures[1] / name / "frames.json", "--tol", options.tol]
            if options.ui:
                check_cmd += ["--ui", options.ui]
            check = tool_verdict(check_cmd)
        write(folder / "check.json", check)
        if all(pair):
            if pair[0]["spec"] != pair[1]["spec"]:
                violations.append("scenario settings changed")
            frames = [{f["key"]: f for f in s["frames"]} for s in pair]
            for key in sorted(set(frames[0]) | set(frames[1])):
                safe_name(key)
                paths = [captures[i] / name / frames[i][key]["path"] if key in frames[i] else None for i in range(2)]
                if None in paths:
                    violations.append(f"missing shot: {key}")
                    continue
                diffdir = folder / key
                command = [options.img_diff, *paths, "--out", diffdir,
                           "--tol", options.tol, "--max-pct", options.max_pct]
                for mask in options.mask:
                    command += ["--mask", mask]
                verdict = tool_verdict(command)
                if not verdict["pass"]:
                    violations.append(f"changed: {key}")
                rows.append(f'<h3>{html.escape(key)} — {"PASS" if verdict["pass"] else "FAIL"}</h3><div class="row">' +
                            "".join(report_image(p, output) for p in [*paths, diffdir / "diff.png"]) + "</div>")
        else:
            violations.append("missing scenario")
        result = {"scenario": name, "pass": check["pass"] and not violations,
                  "differences": violations, "check": check}
        results.append(result)
        sections.append(f'<section><h2>{html.escape(name)}</h2><pre>' +
                        html.escape(json.dumps(result, indent=2)) + "</pre>" + "".join(rows) + "</section>")
    passed = bool(results) and all(r["pass"] for r in results)
    summary = {"format": "shots-compare 1", "base": base, "head": head, "pass": passed,
               "settings": {"tol": options.tol, "max_pct": options.max_pct, "mask": options.mask},
               "scenarios": results}
    write(output / "verdict.json", summary)
    (output / "index.html").write_text('<!doctype html><meta charset="utf-8"><title>Picture checks</title>'
        '<style>body{font:16px system-ui;margin:24px;background:#17202a;color:#eee}.row{display:flex;gap:12px}'
        '.row a{width:33%}img{width:100%}pre{white-space:pre-wrap}section{margin:32px 0}</style>' +
        f'<h1>Picture checks: {"PASS" if passed else "FAIL"}</h1><p>{html.escape(base)} → {html.escape(head)}</p>'
        '<p>Each row: base, head, diff (changed pixels red). Images stay on local disk.</p>' +
        "".join(sections), encoding="utf-8")
    print(output / "index.html")
    return 0 if passed else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("capture")
    c.add_argument("scenarios", help="JSON shot manifest file or directory")
    c.add_argument("--sha", required=True)
    c.add_argument("--bin")
    c.add_argument("--label", help="separate local capture name (repeat experiments)")
    c.add_argument("--game-dir", default=os.environ.get("D2_GAME_DIR"))
    c.add_argument("--timeout", type=float, default=180)
    c.add_argument("--repeats", type=int, default=2)
    p = sub.add_parser("compare")
    p.add_argument("refs", nargs="+", help="[BASE] HEAD capture labels; ancestor default requires HEAD sha")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--img-diff", default="img-diff")
    p.add_argument("--img-check", default="img-check")
    p.add_argument("--tol", type=int, default=0)
    p.add_argument("--max-pct", type=float, default=0)
    p.add_argument("--mask", action="append", default=[])
    p.add_argument("--ui", type=Path)
    for command in (c, p):
        command.add_argument("--root", type=Path, default=GOLDEN)
    options = parser.parse_args(argv)
    try:
        if options.command == "capture":
            if options.repeats < 1 or options.timeout <= 0:
                raise ValueError("positive repeats and timeout required")
            return capture(options)
        if not 0 <= options.tol <= 255 or not 0 <= options.max_pct <= 100:
            raise ValueError("tol must be 0..255; max-pct must be 0..100")
        return compare(options)
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError) as e:
        print(f"shots: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
