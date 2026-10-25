#!/usr/bin/env python3
"""Fresh-process, rotated SQL composition kernel comparisons. No production switches."""
import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess

VARIANTS = {
    "range": ["closure", "demanded-closure", "interval"],
    "demand": ["eager", "selective", "cut"],
    "utility": ["inline", "cte", "cuts"],
    "batch": ["scalar", "stream", "columns"],
}


def summarize(trials):
    medians = {}
    for variant in dict.fromkeys(t["variant"] for t in trials):
        selected = [t for t in trials if t["variant"] == variant]
        medians[variant] = {
            key: statistics.median(t[key] for t in selected)
            for key in ["kernelCpuMs", "commonCpuMs", "totalKernelCpuMs", "peakRssBytes", "tempBytes"]
        }
        for key in ["prepare_cpu_ms", "execute_cpu_ms", "sql_bytes", "statements", "vm_steps", "fullscan_steps", "sorts", "autoindex_rows"]:
            medians[variant][key] = statistics.median(t["sql"][key] for t in selected)
        samples = [t["kernelCpuMs"] for t in selected]
        medians[variant].update(
            sampleCount=len(samples), cpuMinMs=min(samples), cpuMaxMs=max(samples),
            cpuCvPercent=100 * statistics.stdev(samples) / statistics.mean(samples),
        )
    return medians


def run(args):
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=False)
    binary = Path(args.binary).resolve()
    capture = Path(args.capture).resolve()
    variants = VARIANTS[args.experiment]
    reference = None
    trials = []
    for trial in range(args.start_trial, args.start_trial + args.trials):
        order = variants[trial % len(variants):] + variants[:trial % len(variants)]
        for variant in order:
            path = output / f"{trial}-{variant}.json"
            command = [str(binary), str(capture), args.files, args.experiment,
                       variant, str(args.depth), str(path.resolve())]
            subprocess.run(command, check=True, timeout=20)
            data = json.loads(path.read_text())
            rows = data.pop("rows")
            if reference is None:
                reference = rows
            if rows != reference:
                raise RuntimeError(f"exact multiset parity failed: {variant}, trial {trial}")
            data.update(trial=trial, command=command, rowCount=len(rows),
                        rowsSha256=hashlib.sha256(json.dumps(rows, separators=(",", ":")).encode()).hexdigest())
            trials.append(data)
    medians = summarize(trials)
    result = dict(format="rerail.composition-comparison/v1", experiment=args.experiment,
                  files=args.files, depth=args.depth, exactMultisetParity=True,
                  binarySha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  captureSha256=hashlib.file_digest(capture.open("rb"), "sha256").hexdigest(),
                  trials=trials, medians=medians)
    (output / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(medians, indent=2))


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--capture", required=True)
    p.add_argument("--files", default="3")
    p.add_argument("--experiment", choices=VARIANTS, required=True)
    p.add_argument("--depth", type=int, default=6)
    p.add_argument("--start-trial", type=int, default=0)
    p.add_argument("--trials", type=int, default=3)
    p.add_argument("--output", required=True)
    p.add_argument("--binary", default="scripts/profiling/preparation/target/release/composition")
    run(p.parse_args())
