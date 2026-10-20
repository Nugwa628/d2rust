#!/usr/bin/env python3
"""Linux-side replacement for `(Get-Item exe).VersionInfo.FileVersion`: reads the FileVersion /
ProductVersion strings from the exe's VERSIONINFO resource (UTF-16LE key, padding, UTF-16LE value).
    check_pe_version.py <exe> [expected-version]     prints FileVersion; exit 1 on mismatch."""
import re, sys

def field(data, key):
    k = key.encode("utf-16le") + b"\0\0"
    out = []
    i = data.find(k)
    while i >= 0:
        j = i + len(k)
        while data[j:j + 2] == b"\0\0":  # 32-bit alignment padding
            j += 2
        m = re.match(rb"((?:[\x20-\x7e]\x00)+)\x00\x00", data[j:j + 200])
        if m:
            out.append(m.group(1).decode("utf-16le"))
        i = data.find(k, i + 1)
    return out

data = open(sys.argv[1], "rb").read()
fv, pv = field(data, "FileVersion"), field(data, "ProductVersion")
print(f"FileVersion={fv} ProductVersion={pv}")
want = sys.argv[2] if len(sys.argv) > 2 else None
if not fv or not pv or (want and (set(fv) != {want} or set(pv) != {want})):
    sys.exit(f"version resource mismatch (expected {want})")
