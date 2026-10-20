#!/usr/bin/env python3
"""playcheck: checks that need the real game running offscreen, frame by frame.

    python3 tools/playcheck/playcheck.py pacing  --bin EXE --out DIR [--game-dir D] [...]
    python3 tools/playcheck/playcheck.py animgen --bin EXE --out DIR [--game-dir D] [...]
    python3 tools/playcheck/playcheck.py flicker|hover|text --bin EXE --img-check EXE --out DIR [...]
    python3 tools/playcheck/playcheck.py selftest

Each subcommand plays hidden, isolated games (offscreen.py) and writes
`verdict.json` (`pass`, `violations` with the measured values, per-run
details) into a new `--out` folder. Exit 0 pass, 1 a rule broke, 2 bad
arguments or the game could not run. The guide is docs/tools/playcheck.md.

Game files, pictures and logs stay in `--out` on the machine that ran it.
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import animgen  # noqa: E402
import pacing  # noqa: E402
import pictures  # noqa: E402

CHECKS = {"pacing": pacing, "animgen": animgen, "flicker": pictures.Flicker,
          "hover": pictures.Hover, "text": pictures.Text}


def main(argv=None):
    p = argparse.ArgumentParser(prog="playcheck", description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="check", required=True)
    for name, module in CHECKS.items():
        module.add_args(sub.add_parser(name, help=module.__doc__.splitlines()[0]))
    sub.add_parser("selftest", help="the checks' own tests on synthetic data")
    opts = p.parse_args(argv)
    if opts.check == "selftest":
        import unittest
        suite = unittest.defaultTestLoader.discover(str(Path(__file__).resolve().parent), "test_*.py")
        return 0 if unittest.TextTestRunner(verbosity=1).run(suite).wasSuccessful() else 1
    if opts.out.exists():
        print(f"playcheck: {opts.out} exists; give a new folder", file=sys.stderr)
        return 2
    try:
        return CHECKS[opts.check].main(opts)
    except (OSError, RuntimeError) as e:
        print(f"playcheck {opts.check}: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
