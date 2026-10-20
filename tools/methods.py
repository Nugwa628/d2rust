"""Methods collection: docs/METHODS.md.

Each method is `## Mnn Title` followed by exactly these lines, in order:
    - Rule: ...
    - Why: ...
    - Check: ...
    - Here: ...        (binding to this project; dropped by `export`, like Status)
    - Status: proven — <evidence> | trial — <what would prove it>

Usage (from the repo root; Python 3.8+, standard library only):
    py tools/methods.py check            # exit 1 on a malformed entry (CI)
    py tools/methods.py list             # id, status, title
    py tools/methods.py new "Title"      # append a trial entry with the next id
    py tools/methods.py export FILE      # general form (Rule, Why, Check) for another project
"""

import re
import sys
from pathlib import Path

PATH = Path(__file__).resolve().parent.parent / "docs" / "METHODS.md"
FIELDS = ["Rule", "Why", "Check", "Here", "Status"]
HEAD = re.compile(r"^## M(\d{2}) (\S.*)$")
STATUS = re.compile(r"^(proven|trial) — \S")


def parse(text):
    """(preamble lines, [(num, title, {field: value})]) or raises ValueError."""
    lines = text.splitlines()
    preamble, entries, i = [], [], 0
    while i < len(lines) and not lines[i].startswith("## "):
        preamble.append(lines[i])
        i += 1
    while i < len(lines):
        m = HEAD.match(lines[i])
        if not m:
            raise ValueError(f"line {i + 1}: expected '## Mnn Title', got {lines[i]!r}")
        num, title = int(m.group(1)), m.group(2)
        fields = {}
        for k, name in enumerate(FIELDS):
            j = i + 1 + k
            prefix = f"- {name}: "
            if j >= len(lines) or not lines[j].startswith(prefix) or not lines[j][len(prefix):].strip():
                raise ValueError(f"M{num:02}: line {j + 1}: expected '{prefix}...'")
            fields[name] = lines[j][len(prefix):]
        todo = [n for n, v in fields.items() if "TODO" in v]
        if todo:
            raise ValueError(f"M{num:02}: unfinished line(s): {', '.join(todo)}")
        if not STATUS.match(fields["Status"]):
            raise ValueError(f"M{num:02}: Status must start 'proven — ' or 'trial — '")
        entries.append((num, title, fields))
        i += 1 + len(FIELDS)
        while i < len(lines) and not lines[i].strip():
            i += 1
    nums = [e[0] for e in entries]
    if nums != list(range(1, len(nums) + 1)):
        raise ValueError(f"ids must run M01..M{len(nums):02} in order, got {nums}")
    return preamble, entries


def render(entries, general=False):
    """Entry lines; `general` drops the project-bound Here and Status."""
    out = []
    for num, title, f in entries:
        out.append(f"## M{num:02} {title}")
        out += [f"- {n}: {f[n]}" for n in FIELDS if not general or n in ("Rule", "Why", "Check")]
        out.append("")
    return out


def main(argv):
    if not argv or argv[0] not in ("check", "list", "new", "export"):
        print(__doc__)
        return 2
    text = PATH.read_text(encoding="utf-8")
    try:
        preamble, entries = parse(text)
    except ValueError as e:
        print(f"{PATH.name}: {e}", file=sys.stderr)
        return 1
    cmd = argv[0]
    if cmd == "check":
        print(f"{PATH.name}: {len(entries)} methods OK")
    elif cmd == "list":
        for num, title, f in entries:
            print(f"M{num:02}  {f['Status'].split(' ')[0]:6}  {title}")
    elif cmd == "new":
        if len(argv) != 2 or argv[1].startswith("-"):
            print('usage: methods.py new "Title"', file=sys.stderr)
            return 2
        num = len(entries) + 1
        stub = {n: "TODO" for n in FIELDS}
        stub["Status"] = "trial — TODO"
        entries.append((num, argv[1], stub))
        PATH.write_text("\n".join(preamble + render(entries)).rstrip() + "\n", encoding="utf-8", newline="\n")
        print(f"added M{num:02} {argv[1]}: fill in its lines")
    else:
        if len(argv) != 2:
            print("usage: methods.py export FILE", file=sys.stderr)
            return 2
        head = [
            "# Methods",
            "",
            "General working methods. In a new project, add after each `- Check:`",
            "line a `- Here:` line binding the method to that project (its reference,",
            "tools and files) and a `- Status: trial — <what would prove it>` line;",
            "a method becomes `proven — <evidence>` only once shown there.",
            "",
        ]
        Path(argv[1]).write_text("\n".join(head + render(entries, general=True)).rstrip() + "\n", encoding="utf-8", newline="\n")
        print(f"wrote {argv[1]} ({len(entries)} methods)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
