"""Print phase medians for one or more join lab reports as TSV."""

import argparse
import csv
import json
from pathlib import Path
from statistics import median
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", type=Path, nargs="+")
    args = parser.parse_args()
    summary = {}
    for path in args.reports:
        record = json.loads(path.read_text())
        for case in record["cases"]:
            tag = case["name"]
            if "corpus" in case:
                tag += "-" + str(case["corpus"]["files"])
            for run in case.get("runs", []):
                for phase in run["rounds"]:
                    key = (tag, phase["phase"], run["mode"])
                    values = [
                        phase["cpu_ms"],
                        phase["wall_ms"],
                        phase["vm_steps_approx"],
                        run["peak_rss_mib"],
                        phase["rows"],
                        phase["wide_rows_written"],
                        phase["factor_rows_written"],
                        phase["cache_hits"],
                        phase["negative_hits"],
                    ]
                    summary.setdefault(key, []).append(values)
    writer = csv.writer(sys.stdout, delimiter="\t", lineterminator="\n")
    writer.writerow(
        [
            "case",
            "phase",
            "mode",
            "samples",
            "cpu_ms",
            "wall_ms",
            "approx_vm_steps",
            "process_peak_mib",
            "rows_returned",
            "wide_rows_written",
            "factor_rows_written",
            "cache_hits",
            "negative_hits",
        ]
    )
    for key, samples in summary.items():
        writer.writerow(
            [*key, len(samples), *(median(values) for values in zip(*samples))]
        )


if __name__ == "__main__":
    main()
