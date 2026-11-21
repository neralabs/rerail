"""Compare the native TRY401 plans on the pinned PostHog sample."""

import argparse
import csv
import json
import os
from pathlib import Path
import tempfile

import corpus


POLICY = (
    corpus.ROOT / "docs/rule-syntax/packs/python/ruff-try401-verbose-log-message.rerail"
)
SAMPLE_DIGEST = "b7821c1a80078968c0fb5ae5b66b17f9d636a68fae6a3638befb16676ad73a5c"
DIAGNOSTIC_DIGEST = "6cf0d92b4063e6197bd8e1ebd72bad7a40efb1b024cc733f198886fbd5bd7eb6"
PROBE = """import logging

def probe():
    try:
        int("probe")
    except ValueError as err:
        logging.exception("failed: %s", err)
"""


def compare(engine, sources, out, stores):
    records = {}

    def run(name, mode, store, profiled=False):
        result = corpus.native(
            engine,
            sources,
            out / name,
            stores / store,
            mode=mode,
            policy=POLICY,
            plan=profiled,
            profile=profiled,
        )
        records[name] = result
        (out / "results.json").write_text(json.dumps(records, indent=2) + "\n")
        print(name, result, flush=True)
        return result["sha256"]

    reference = run("cold-off-1", "off", "off-1")
    if reference != DIAGNOSTIC_DIGEST:
        raise ValueError("the baseline differs from the pinned TRY401 finding")
    for name, mode in [("on-1", "on"), ("on-2", "on"), ("off-2", "off")]:
        if run("cold-" + name, mode, name) != reference:
            raise ValueError(f"{name} changed the cold diagnostics")

    probe = sources / "posthog/_deferred_join_try401_probe.py"
    with probe.open("x") as source:
        source.write(PROBE)
    try:
        fresh = run("edit-cold", "off", "control")
        for mode in ("off", "on"):
            if run("insert-" + mode, mode, mode + "-2") != fresh:
                raise ValueError(f"{mode} disagrees with the fresh edit")
        diagnostics = json.loads((out / "edit-cold/report.json").read_text())[
            "diagnostics"
        ]
        if not any(r["path"].endswith(probe.name) for r in diagnostics):
            raise ValueError("the edit did not trigger TRY401")
    finally:
        probe.unlink()
    for mode in ("off", "on"):
        if run("delete-" + mode, mode, mode + "-2") != reference:
            raise ValueError(f"{mode} disagrees after removal")

    for mode in ("off", "on"):
        name = "profile-" + mode
        if run(name, mode, name, profiled=True) != reference:
            raise ValueError(f"{mode} changed diagnostics with profiling")
        with (out / name / "profile/native.tsv").open() as f:
            plans = [r["plan"] for r in csv.DictReader(f, delimiter="\t")]
        factors = [s for s in plans if "fz" in s and "_exc_qual(" in s]
        if bool(factors) != (mode == "on"):
            raise ValueError(f"unexpected exc_qual plan in {mode}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--git", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(dir=out) as tmp:
        stores = Path(tmp)
        sources = stores / "sources"
        if corpus.prepare(args.git, 600, sources) != SAMPLE_DIGEST:
            raise ValueError("pinned source digest differs")
        compare(args.engine.resolve(strict=True), sources, out, stores)


if __name__ == "__main__":
    os.nice(19)
    main()
