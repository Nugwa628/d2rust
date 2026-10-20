#!/usr/bin/env python3
"""Merge numeric coverage from repro/explore/sweep reports; list untouched rows.

Pass individual reports, or directories containing repro.json. Do not combine
an aggregate sweep report with its member reports (that would count them twice).
Game-table strings and assets are neither needed nor written.
"""
import argparse
import json
from pathlib import Path


def merge(reports):
    counters, tables = {}, {}
    for report in reports:
        coverage = report.get("coverage", report)
        if coverage.get("format") != "coverage 1":
            raise ValueError("expected coverage 1 envelope")
        for category, rows in coverage.get("tables", {}).items():
            if not all(isinstance(row, str) for row in rows):
                raise ValueError("table row IDs must be strings")
            tables.setdefault(category, set()).update(rows)
        for category, rows in coverage.get("counters", {}).items():
            dest = counters.setdefault(category, {})
            for row, count in rows.items():
                if type(count) is not int or count < 0:
                    raise ValueError("coverage counts must be nonnegative integers")
                dest[row] = dest.get(row, 0) + count
    never = {category: sorted(rows - {row for row, count in counters.get(category, {}).items() if count},
                                key=lambda row: (int(row) if row.isdecimal() else -1, row))
             for category, rows in sorted(tables.items())}
    return {"format": "coverage 1", "counters": counters,
            "tables": {k: sorted(v) for k, v in sorted(tables.items())},
            "never_reached": never,
            "summary": {k: {"total": len(tables[k]), "never_reached": len(v),
                            "reached": len(tables[k]) - len(v)} for k, v in never.items()}}


def paths(inputs):
    found = set()
    for value in inputs:
        path = Path(value)
        if path.is_dir():
            files = sorted(path.rglob("repro.json"))
            if not files:
                files = sorted(path.rglob("explore.json")) or sorted(path.rglob("sweep.json"))
            found.update(p.resolve() for p in files)
        else:
            found.add(path.resolve())
    if not found:
        raise ValueError("no coverage reports found")
    return sorted(found)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    result = merge(json.loads(p.read_text(encoding="utf-8")) for p in paths(args.reports))
    text = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
