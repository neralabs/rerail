"""Generate the native Python word table offline with the pinned CPython version."""
from pathlib import Path
import sys
import unicodedata

if sys.version_info[:2] != (3, 13) or unicodedata.unidata_version != "15.1.0":
    raise SystemExit("Use CPython 3.13 with Unicode 15.1.0")

root = Path(__file__).resolve().parents[3]
output = root / "native/src/code_rule_dsl/value/python/words.rs"
ranges, start = [], None
for point in range(0x110001):
    word = point < 0x110000 and (chr(point).isalnum() or point == 95)
    if word and start is None:
        start = point
    if not word and start is not None:
        ranges.append((start, point - 1))
        start = None
marker = "// Sorted inclusive ranges; surrogates and punctuation are absent."
prefix, _ = output.read_text().split(marker, 1)
output.write_text(prefix + marker + "\nconst RANGES: &[(u32, u32)] = &[\n" +
    "".join(f"    (0x{a:x}, 0x{b:x}),\n" for a, b in ranges) + "];\n")
print(f"Generated {len(ranges)} Python word ranges")
