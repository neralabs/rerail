"""Compare CPU, RSS and complete finding multisets on identical upstream/native snapshots."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path


def finding_keys(events):
    return Counter(json.dumps(["data-layer-" + event["check"].replace("_", "-"), row["path"], row["line"], row["code"]],
                              separators=(",", ":")) for event in events for row in event["findings"])


def workload_digest(recording):
    # Match suite.ts after its schema has stripped recording-only metadata.
    value = {"commit": recording["commit"],
             "files": [{"path": row["path"], "source": row["source"]} for row in recording["files"]],
             "events": [{"check": event["check"], "findings": [{key: row[key] for key in ("path", "line", "code")}
                         for row in event["findings"]]} for event in recording["events"]]}
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()


def compare_snapshot(profile, recording, oracle):
    native = profile["parity"]
    if oracle["workers"] != 1 or oracle.get("profiled", False):
        raise ValueError("Use an unprofiled single-worker oracle for CPU/RSS comparison")
    if profile.get("trace", "off") != "off" or profile.get("external", "none") != "none" or profile.get("memoryTool", "none") != "none" or profile.get("memoryCensus", False) or profile.get("sqlProfile", False):
        raise ValueError("Use a native control run without diagnostic capture")
    if profile["timedOut"] or profile["exitCode"] not in (0, 1) or native["failures"] or native["incrementalDifferences"]:
        raise ValueError("A completed native evaluation without engine failures is required")
    if not native["commit"] == recording["commit"] == oracle["commit"] or oracle["source_mismatches"]:
        raise ValueError("Source pins do not match")
    if native.get("workloadSha256") != workload_digest(recording):
        raise ValueError("Original recording does not match the timed workload fingerprint")
    identity = lambda snapshot: sorted((row["path"], hashlib.sha256(row["source"].encode()).hexdigest()) for row in snapshot["files"])
    if identity(recording) != identity(oracle):
        raise ValueError("Source files differ; include project metadata sentinels in both snapshots")
    if {row["check"] for row in recording["events"]} != {row["check"] for row in oracle["events"]}:
        raise ValueError("Rule sets do not match")
    original = finding_keys(recording["events"])
    missing = Counter(native["falseNegatives"])
    if sum(original.values()) != native["expected"] or missing - original:
        raise ValueError("Original native expectations do not match the supplied recording")
    actual = original - missing + Counter(native["falsePositives"])
    if sum(actual.values()) != native["actual"]:
        raise ValueError("Recorded native finding counts are inconsistent")
    expected = finding_keys(oracle["events"])
    extra, absent = actual - expected, expected - actual
    native_cpu = sum(native["phaseCpuMs"]["cold"].values())
    python_cpu = oracle.get("oracle_total_cpu_ms", oracle.get("oracle_cpu_ms"))
    return {"passed": not extra and not absent, "files": len(recording["files"]),
            "bytes": sum(len(row["source"].encode()) for row in recording["files"]),
            "checks": sorted(row["check"] for row in oracle["events"]),
            "expected": sum(expected.values()), "actual": sum(actual.values()),
            "falsePositives": sorted(extra.elements()), "falseNegatives": sorted(absent.elements()),
            "nativeColdCpuMs": native_cpu, "pythonCpuMs": python_cpu,
            "nativeToPythonCpuRatio": native_cpu / python_cpu if python_cpu else None,
            "nativePeakRssBytes": native["peakRssBytes"], "pythonPeakRssBytes": oracle["peak_rss_bytes"],
            "nativeToPythonRssRatio": native["peakRssBytes"] / oracle["peak_rss_bytes"],
            "nativeOriginalExitCode": profile["exitCode"], "nativeSha256": native["nativeSha256"],
            "retainedReporterContracts": oracle.get("retained_reporter_contracts", []),
            "note": "Native actual findings are reconstructed from the timed run's original expectations and complete FP/FN multisets. The original report is unchanged. CPU excludes native policy compilation. Python includes the source adapter. RSS includes process setup; both runs must use one process for RSS comparison."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("native_directory", type=Path)
    parser.add_argument("python_oracle", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    profile_path = args.native_directory / "profile.json"
    recording_path = args.native_directory / "selected-recording.json"
    result = compare_snapshot(json.loads(profile_path.read_text()), json.loads(recording_path.read_text()),
                              json.loads(args.python_oracle.read_text()))
    result["evidenceSha256"] = {str(path): hashlib.sha256(path.read_bytes()).hexdigest()
                                for path in (profile_path, recording_path, args.python_oracle)}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
