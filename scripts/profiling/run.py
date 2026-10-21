#!/usr/bin/env python3
"""Bounded native profiling with optional macOS sampling and checked corpus subsets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time

from report import LEAF_PHASES, aggregate, chrome_trace, summary, phase_overlap
from sample import bottom_up

ROOT = Path(__file__).resolve().parents[2]


def select_recording(recording, selection):
    if selection["commit"] != recording["commit"] or recording["source_mismatches"]:
        raise ValueError("Profile source commit does not match the verified recording")
    wanted = {file["path"]: file["sha256"] for file in selection["files"]}
    if not wanted or len(wanted) != len(selection["files"]) or not selection["checks"] or len(set(selection["checks"])) != len(selection["checks"]):
        raise ValueError("Profile selection requires distinct files and checks")
    files = [file for file in recording["files"] if file["path"] in wanted]
    if len(files) != len(wanted):
        raise ValueError("Profile selection has missing or duplicate files")
    for file in files:
        if hashlib.sha256(file["source"].encode()).hexdigest() != wanted[file["path"]]:
            raise ValueError(f"Profile source hash mismatch: {file['path']}")
    events = [{**event, "findings": [row for row in event["findings"] if row["path"] in wanted]}
              for event in recording["events"] if event["check"] in selection["checks"]]
    if {event["check"] for event in events} != set(selection["checks"]):
        raise ValueError("Profile selection has missing checks")
    return {**recording, "files": files, "events": events, "oracle_ms": None,
            "profileNote": "Oracle timing omitted: selected findings come from the pinned full recording."}


def recording_from_source(source, selection):
    root = source.resolve()
    files = []
    for item in selection["files"]:
        path = (root / item["path"]).resolve()
        if not path.is_relative_to(root):
            raise ValueError("Profile path is outside the source root")
        files.append({"path": item["path"], "source": path.read_text()})
    recording = {"commit": selection["commit"], "workers": 1, "oracle_ms": None, "source_mismatches": [],
                 "files": files, "events": [{"check": check, "findings": selection["expectations"][check]} for check in selection["checks"]]}
    return select_recording(recording, selection)


def kill_group(process):
    if process.poll() is None:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGKILL)
        else:
            process.kill()
        process.wait(timeout=2)


def execute(args, command, output, deadline):
    env = {**os.environ, "RERAIL_PROFILE_PHASES": "1"}
    env.pop("RERAIL_CODE_RULE_ANCHORS", None)
    env.pop("RERAIL_CODE_RULE_RELATIONAL_PROFILE", None)
    env.pop("RERAIL_PROFILE", None)
    env.pop("RERAIL_SQL_QUERY_PLANS", None)
    env.pop("RERAIL_SQL_PROFILE_DIR", None)
    env.pop("RERAIL_SQL_COMPLETED_OUTPUT", None)
    if args.completed_output == "virtual":
        env["RERAIL_SQL_COMPLETED_OUTPUT"] = "virtual"
    if args.sql_profile:
        raw = output / "sql" / "raw"
        raw.mkdir(parents=True)
        env["RERAIL_SQL_PROFILE_DIR"] = str(raw)
    env["RERAIL_SQL_INSERT_BATCH"] = str(args.insert_batch)
    env.pop("RERAIL_SQL_INPUT_VTAB", None)
    if args.native_input != "table":
        env["RERAIL_SQL_INPUT_VTAB"] = args.native_input
    env.pop("RERAIL_SQL_RETAIN_STAGES", None)
    if args.retain_sql_stages:
        env["RERAIL_SQL_RETAIN_STAGES"] = "1"
    env.pop("RERAIL_SHARED_INPUT_INDEX", None)
    if args.shared_input_index:
        env["RERAIL_SHARED_INPUT_INDEX"] = "1"
    if args.query_plans:
        env["RERAIL_SQL_QUERY_PLANS"] = "1"
    if args.trace == "on":
        env["RERAIL_PROFILE"] = "1"
    samples = []
    timed_out = False
    with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open("w") as stderr:
        process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr,
                                   env=env, start_new_session=os.name == "posix")
        next_sample = time.monotonic() + args.sample_every
        try:
            while process.poll() is None:
                remaining = deadline - time.monotonic()
                if remaining < 3:
                    timed_out = True
                    kill_group(process)
                    break
                if args.external == "sample" and len(samples) < args.sample_count and time.monotonic() >= next_sample and remaining > args.sample_seconds + 8:
                    raw = output / f"sample-{len(samples) + 1}.txt"
                    started = time.time()
                    with (output / "sampler.log").open("a") as log:
                        sampler = subprocess.Popen(["/usr/bin/sample", str(process.pid), str(args.sample_seconds), "1", "-file", str(raw)],
                                                   stdout=log, stderr=log, start_new_session=True)
                        try:
                            sampler.wait(timeout=min(args.sample_seconds + 6, remaining - 3))
                        except subprocess.TimeoutExpired:
                            log.write("Sampler exceeded its budget; terminating it.\n")
                        finally:
                            kill_group(sampler)
                    try:
                        converted = bottom_up(raw.read_text(), args.sample_module or ["rerail_engine"]) if raw.exists() else {}
                    except ValueError as error:
                        converted = {"parseError": str(error)}
                    samples.append({"file": raw.name, "startUnixUs": int(started * 1e6),
                                    "captureSeconds": args.sample_seconds,
                                    "exitCode": sampler.returncode, **converted})
                    next_sample = time.monotonic() + args.sample_every
                time.sleep(min(0.05, max(0, remaining - 3)))
        finally:
            kill_group(process)
    return process.returncode, timed_out, samples


def main():
    started = time.monotonic()
    load_start = os.getloadavg() if hasattr(os, "getloadavg") else None
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--recording", type=Path)
    parser.add_argument("--source", type=Path, help="Pinned data-layer source directory; requires a selection with recorded expectations")
    parser.add_argument("--selection", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--deadline", type=float, default=110, help="Whole command budget in seconds, maximum 115")
    parser.add_argument("--external", choices=["none", "sample"], default="none")
    parser.add_argument("--trace", choices=["on", "off"], default="on")
    parser.add_argument("--cold-only", action="store_true", help="Skip unchanged and edit phases in the data-layer driver")
    parser.add_argument("--insert-batch", type=int, default=32, choices=range(1,257), metavar="1..256", help="SQLite input batch row cap (default 32; profiling builds allow 1 as a control)")
    parser.add_argument("--shared-input-index", action="store_true", help="Experimental document-scoped kind postings shared between input selectors")
    parser.add_argument("--query-plans", action="store_true", help="Record SQLite EXPLAIN QUERY PLAN for materialized stages (requires tracing)")
    parser.add_argument("--sql-profile", action="store_true", help="Capture every executed relational SQLite statement, plans, bytecode, counters and HTML report (requires code-rule-profile)")
    parser.add_argument("--completed-output", choices=["table", "virtual"], default="table", help="Profile-only completed producer publication adapter")
    parser.add_argument("--sample-every", type=float, default=6)
    parser.add_argument("--sample-count", type=int, default=4)
    parser.add_argument("--sample-seconds", type=int, default=1)
    parser.add_argument("--sample-module", action="append", help="Include stacks with this module or symbol substring; repeat, or use '*' for all active stacks")
    parser.add_argument("--native-input", choices=["table", "rows", "captures"], default="table", help="Profile-only native row virtual table instead of SQL input insertion")
    parser.add_argument("--retain-sql-stages", action="store_true", help="Profile-only control: retain intermediate SQL stages until the connection closes")
    parser.add_argument("--allocator", default="unspecified")
    parser.add_argument("--policies", default="packs/data-layer/candidates/rules")
    parser.add_argument("command", nargs=argparse.REMAINDER, help="Optional command after --, instead of a data-layer recording")
    args = parser.parse_args()
    if not 5 <= args.deadline <= 115 or not 1 <= args.sample_seconds <= 5 or args.sample_every < 0.1 or not 0 <= args.sample_count <= 10:
        parser.error("Invalid budget or sampling bounds; --deadline must be between 5 and 115 seconds")
    if args.external == "sample" and sys.platform != "darwin":
        parser.error("--external sample requires macOS; use --external none with another external profiler")
    if args.query_plans and args.trace != "on":
        parser.error("--query-plans requires --trace on")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if args.recording and args.source:
        parser.error("Choose --source or --recording")
    if args.source and not args.selection:
        parser.error("--source requires --selection")
    if args.recording or args.source:
        if command:
            parser.error("Choose a recording or a command")
        recording = args.recording.resolve() if args.recording else None
        if args.selection:
            selection = json.loads(args.selection.read_text())
            (output / "selection.json").write_text(json.dumps(selection, indent=2) + "\n")
            selected = recording_from_source(args.source, selection) if args.source else select_recording(json.loads(recording.read_text()), selection)
            recording = output / "selected-recording.json"
            recording.write_text(json.dumps(selected))
        command = ["node", "--import", "tsx", "conformance/data-layer/suite.ts", "--recording", str(recording),
                   "--output", str(output / "parity.json"), "--allocator", args.allocator, "--policies", args.policies]
        if args.cold_only:
            command.append("--cold-only")
    if not command:
        parser.error("Supply --recording or a command after --")
    code, timed_out, samples = execute(args, command, output, started + args.deadline)
    native, events = aggregate((output / "stderr.log").read_text())
    for sample in samples:
        start = sample["startUnixUs"]
        end = start + sample["captureSeconds"] * 1e6
        sample["approximateDriverPhaseOverlap"] = phase_overlap(native["phaseMarkers"], start, end)
        overlaps = []
        for event in events:
            if event["phase"] not in LEAF_PHASES:
                continue
            overlap = min(end, event["startUnixUs"] + event["wallNs"] / 1000) - max(start, event["startUnixUs"])
            if overlap > 0:
                overlaps.append({"runPhase": event["runPhase"], "phase": event["phase"], "name": event["name"],
                                 "clause": event["clause"], "path": event["path"], "overlapMs": overlap / 1000})
        sample["approximateSpanOverlap"] = sorted(overlaps, key=lambda row: -row["overlapMs"])[:10]
    (output / "trace.jsonl").write_text("".join(json.dumps(event) + "\n" for event in events))
    (output / "trace.chrome.json").write_text(json.dumps(chrome_trace(events)) + "\n")
    report = {"version": 1, "command": command, "elapsedSeconds": time.monotonic() - started,
              "exitCode": code, "timedOut": timed_out, "deadlineSeconds": args.deadline,
              "trace": args.trace, "external": args.external, "queryPlans": args.query_plans, "sqlProfile":args.sql_profile, "completedOutput":args.completed_output, "insertBatch": args.insert_batch, "nativeInput": args.native_input, "sharedInputIndex": args.shared_input_index, "retainSqlStages":args.retain_sql_stages,
              "sampleModules":args.sample_module or ["rerail_engine"],
              "host": {"platform": platform.platform(), "cpu": platform.processor(), "python": platform.python_version(),
                       "logicalCpus": os.cpu_count(), "loadAverageStart": load_start,
                       "loadAverageEnd": os.getloadavg() if hasattr(os, "getloadavg") else None},
              "native": native, "samples": samples}
    if (output / "parity.json").exists():
        report["parity"] = json.loads((output / "parity.json").read_text())
    (output / "profile.json").write_text(json.dumps(report, indent=2) + "\n")
    if args.sql_profile:
        remaining = args.deadline - (time.monotonic() - started)
        try:
            with (output / "sql-report.log").open("w") as log:
                subprocess.run(["node", "scripts/profiling/sql-report.mjs", str(output)], cwd=ROOT,
                               stdout=log, stderr=subprocess.STDOUT, timeout=max(0.01, remaining - 0.5), check=True)
        except (subprocess.SubprocessError, OSError) as error:
            report["sqlProfileError"] = str(error)
            code = code or 1
            timed_out = timed_out or isinstance(error, subprocess.TimeoutExpired)
        report.update(exitCode=code, timedOut=timed_out, elapsedSeconds=time.monotonic() - started)
        (output / "profile.json").write_text(json.dumps(report, indent=2) + "\n")
    (output / "summary.txt").write_text(summary(report))
    print(json.dumps({"output": str(output), "seconds": round(report["elapsedSeconds"], 3), "timedOut": timed_out,
                      "exitCode": code, "traceEvents": native["traceEvents"], "samples": len(samples)}))
    if args.trace == "on" and native["traceEvents"] == 0:
        print("No attributed spans recorded. Build native with --features code-rule-profile and use a relational rule.", file=sys.stderr)
    if native["parseErrors"]:
        print(f"Warning: {len(native['parseErrors'])} malformed profiling lines; retained valid evidence and stderr.log.", file=sys.stderr)
    return 124 if timed_out else code if code else 1 if any(sample["exitCode"] != 0 or sample.get("parseError") for sample in samples) else 0


if __name__ == "__main__":
    raise SystemExit(main())
