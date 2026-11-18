#!/usr/bin/env python3
"""Writes native/crates/json_lean/src/tables.rs: simdjson's own tables, copied from the vendored
amalgamation (native/vendor/simdjson-5.0.5/simdjson.cpp), so the translation reads the same values.
Run from the repository root."""
import re, sys
src = open("native/vendor/simdjson-5.0.5/simdjson.cpp").read()

def body(start):
    i = src.index(start)
    j = src.index("{", i) + 1
    k = src.index("};", j)
    text = re.sub(r"//[^\n]*", "", src[j:k])
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return [t.strip() for t in text.split(",") if t.strip()]

tables = [
    ("STRUCTURAL_OR_WHITESPACE_NEGATED", "u8", "const bool structural_or_whitespace_negated[256] = {", 256),
    ("DIGIT_TO_VAL32", "u32", "const uint32_t digit_to_val32[886] = {", 886),
    ("POWER_OF_FIVE_128", "u64", "const uint64_t simdjson::internal::power_of_five_128[]= {", None),
    ("ESCAPE_MAP", "u8", "static const uint8_t escape_map[256] = {", 256),
]
out = ["//! simdjson 5.0.5's tables, copied from its amalgamation by `scripts/json_lean/tables.py`. Generated: do",
       "//! not edit.", ""]
for name, ty, start, n in tables:
    vals = body(start)
    if n is not None:
        assert len(vals) == n, (name, len(vals))
    if ty == "f64":
        vals = [v if "." in v or "e" in v else v + ".0" for v in vals]
    out.append(f"pub static {name}: [{ty}; {len(vals)}] = [")
    for i in range(0, len(vals), 8):
        out.append("    " + ", ".join(vals[i:i + 8]) + ",")
    out.append("];")
    out.append("")
open("native/crates/json_lean/src/tables.rs", "w").write("\n".join(out))
print("tables.rs:", ", ".join(f"{n}" for n, *_ in tables))
