# Animation frame pack v2

A bake folder holds the generated in-between animation frames of one game or
mod build. Packs are baked once per build ([the bake](animation-generation.md))
and shipped as ordinary files; the repository and releases contain no such
packs. Mods are baked the same way at install, or their own pipeline runs the
shared generator. Their assets and data stay outside this repository.

```
<bake folder>/
  bake.json          the manifest, written last
  blend.lut          196,608 bytes: three 256x256 colour-pair tables
  packs/<identity>.d2fz
```

All integers are little endian. A pack file is a raw deflate stream of the
bytes below. The header is the eight bytes `D2FRAME\0`, a `u16` version (2),
the 32-byte SHA-256 input identity, a `u8` looping flag, and a `u8` direction
entry count. An unknown version is refused.

Each direction entry is:

| Field | Encoding |
|---|---|
| Direction | `u8`, 0–63, ascending, no duplicates |
| Frame count | `u16`, at least 1 |
| Per source frame | `dx i8`, `dy i8`, accepted-fraction mask `u8` |

Mask bit 0 is fraction 64, bit 1 is 128 and bit 2 is 192; a set bit means that
in-between was accepted by the quality gates. Other bits are refused. The file
ends with a `u32` accepted-frame count, which must equal the sum of set mask
bits. Source pairs are contiguous frames of each direction; the cel after the
last is the first when the looping flag is set, else the last cel itself.

`blend.lut` holds three 65,536-byte tables in fraction order 64, 128, 192.
Index `pa * 256 + pb` is the output palette index for a pair of source palette
indices (earlier cel `pa`, later cel `pb`) at that fraction. It already
contains the deterministic rules (transparent pairs pick `pa` for fraction 64
and `pb` for 128/192; incompatible ramps do the same; compatible pairs take the
nearest palette colour on the starting ramp). The loader remaps source pixels
through this table; it performs no motion search, colour math or gate decision.

The pack identity is the SHA-256 of the source pictures, palette, colour-map
ramps, generator settings and pack format version. It deliberately excludes
the archive file name, so byte-identical component files under different names
share one content-addressed pack file. A changed source, palette, ramp set or
generator changes the identity and the file name. Each pack's identity is
checked against the player's source file before its pictures are used.

`bake.json` lists the folder's packs with their component file and key, and
the SHA-256 of `blend.lut`; see the game's `publish/FORMAT.md` for the exact
manifest and publishing rules. Decoding validates all counts and lengths before
copying or rebuilding pixels, within the 64 MiB per-pack, 16,384 accepted-frame
and 4,194,304 pixels per picture limits. Offsets stay within +/-1,048,576
pixels. The mutation regression verifies that every accepted mutation
round-trips through the canonical encoder.
