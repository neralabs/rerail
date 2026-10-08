"""Regression coverage for percent-format width probes without materializing their output."""

from __future__ import annotations

import importlib.util
import json
import resource
from pathlib import Path

HERE = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("generate_percent", HERE / "generate_percent.py")
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

CASES = {"%2147483647d", "%2147483648d", "%99999999999d"}
EXPECTED = {
    row[0]: row[1]
    for row in json.loads((HERE / "percent.expected.json").read_text())["cases"]
    if row[0] in CASES
}

# Keep the probe below the format widths; it runs in Linux CI where RLIMIT_AS is enforced.
resource.setrlimit(resource.RLIMIT_AS, (1 << 30, 1 << 30))

assert EXPECTED.keys() == CASES
for case in CASES:
    assert MODULE.cpython(case) == EXPECTED[case]
