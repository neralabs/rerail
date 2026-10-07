"""Writes names.bin, the Unicode character names `\\N{...}` reads, as CPython 3.13 (Unicode 15.1.0) reads them.

Stored names are every name unicodedata.name gives (except the CJK unified ideographs and Hangul syllables, whose
names the decoder derives) and every alias in the UCD's NameAliases.txt, which `\\N{...}` accepts and
unicodedata.name does not give. Each one is checked against CPython's own `\\N{...}`. Run offline, not in CI:
python3.13 generate.py <path to NameAliases.txt of Unicode 15.1.0>

Layout, little-endian:
- b"TNAMES01", then the Unicode version as one length byte and ASCII;
- the CJK unified ideograph ranges: a u16 count, then (first, last) u32 pairs;
- the stored names, sorted: a u32 count, a u32 bucket count, then each bucket's u32 offset into the entries;
- the entries: a shared-prefix length (0 at each bucket's first entry), a suffix length, the suffix (upper-case
  ASCII), and the code point in three bytes.
"""

from __future__ import annotations

import pathlib
import struct
import sys
import unicodedata

HERE: pathlib.Path = pathlib.Path(__file__).parent
BUCKET: int = 16
CJK: str = "CJK UNIFIED IDEOGRAPH-"
HANGUL: str = "HANGUL SYLLABLE "


def named(name: str) -> str | None:
    try:
        return eval('"\\N{%s}"' % name)  # noqa: S307 - the names are the UCD's own
    except SyntaxError:
        return None


def main() -> None:
    if sys.version_info[:2] != (3, 13) or unicodedata.unidata_version != "15.1.0":
        raise SystemExit(f"run with CPython 3.13 (Unicode 15.1.0), not {sys.version} ({unicodedata.unidata_version})")
    names: dict[str, int] = {}
    cjk: list[int] = []
    for code in range(0x110000):
        name: str = unicodedata.name(chr(code), "")
        if not name:
            continue
        if name.startswith(CJK):
            cjk.append(code)
        elif not name.startswith(HANGUL):
            names[name] = code
    for line in pathlib.Path(sys.argv[1]).read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        code_text, alias, _kind = line.split(";")
        names.setdefault(alias, int(code_text, 16))
    for name, code in names.items():
        if not name.isascii() or name != name.upper() or named(name) != chr(code):
            raise SystemExit(f"{name!r} does not name U+{code:04X} in CPython")
    ranges: list[tuple[int, int]] = []
    for code in cjk:
        if ranges and ranges[-1][1] == code - 1:
            ranges[-1] = (ranges[-1][0], code)
        else:
            ranges.append((code, code))
    for first, last in ranges:
        if named(f"{CJK}{first:04X}") != chr(first) or named(f"{CJK}{last:04X}") != chr(last):
            raise SystemExit(f"CPython does not name the CJK range {first:04X}..{last:04X}")
    items: list[tuple[str, int]] = sorted(names.items())
    entries: bytearray = bytearray()
    offsets: list[int] = []
    previous: str = ""
    for index, (name, code) in enumerate(items):
        shared: int = 0
        if index % BUCKET == 0:
            offsets.append(len(entries))
        else:
            while shared < min(len(name), len(previous), 255) and name[shared] == previous[shared]:
                shared += 1
        suffix: bytes = name[shared:].encode("ascii")
        entries += bytes([shared, len(suffix)]) + suffix + code.to_bytes(3, "little")
        previous = name
    version: bytes = unicodedata.unidata_version.encode("ascii")
    out: bytearray = bytearray(b"TNAMES01") + bytes([len(version)]) + version
    out += struct.pack("<H", len(ranges))
    for first, last in ranges:
        out += struct.pack("<II", first, last)
    out += struct.pack("<II", len(items), len(offsets))
    for offset in offsets:
        out += struct.pack("<I", offset)
    out += entries
    (HERE / "names.bin").write_bytes(bytes(out))
    print(f"{len(items)} names, {len(ranges)} CJK ranges, {len(out)} bytes")


if __name__ == "__main__":
    main()
