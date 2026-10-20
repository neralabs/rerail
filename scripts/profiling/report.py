"""Aggregate attributed native spans. Inclusive spans must not be added to children."""
import collections
import json

LEAF_PHASES = {"input-extract", "sql-input", "sql-validate", "sql-materialize", "sql-index", "sql-analyze", "sql-result", "sql-reach-prepare", "sql-reach-walk", "sql-reach-index"}


def aggregate(log):
    phase = "startup"
    grouped = {}
    applies, events, parse_errors, markers = [], [], [], []
    for number, line in enumerate(log.splitlines(), 1):
        if not line.startswith(("RERAIL_PROFILE_PHASE ", "RERAIL_CODE_RULE_PROFILE ", "RERAIL_TRACE ")):
            continue
        try:
            payload = json.loads(line.split(" ", 1)[1])
        except ValueError as error:
            parse_errors.append({"line": number, "prefix": line.split(" ", 1)[0], "error": str(error)})
            continue
        if line.startswith("RERAIL_PROFILE_PHASE "):
            phase = payload["phase"]
            markers.append(payload)
        elif line.startswith("RERAIL_CODE_RULE_PROFILE "):
            applies.append({"phase": phase, **payload})
        elif line.startswith("RERAIL_TRACE "):
            event = {"runPhase": phase, **payload}
            events.append(event)
            key = (phase, event["rule"], event["clause"], event["path"], event["phase"], event["name"])
            row = grouped.setdefault(key, {"runPhase": phase, "rule": event["rule"], "clause": event["clause"],
                "path": event["path"], "phase": event["phase"], "name": event["name"], "calls": 0,
                "wallMs": 0, "cpuMs": 0, "cpuMeasuredCalls": 0, "maxWallMs": 0, "counters": collections.Counter()})
            row["calls"] += 1
            row["wallMs"] += event["wallNs"] / 1e6
            row["maxWallMs"] = max(row["maxWallMs"], event["wallNs"] / 1e6)
            if event["cpuNs"] is not None:
                row["cpuMs"] += event["cpuNs"] / 1e6
                row["cpuMeasuredCalls"] += 1
            for name, value in (event.get("detail") or {}).items():
                if isinstance(value, (int, float)):
                    row["counters"][name] += value
    return {"traceEvents": len(events), "applies": applies, "parseErrors": parse_errors, "phaseMarkers": markers,
            "spans": sorted(grouped.values(), key=lambda row: -row["wallMs"])}, events


def phase_overlap(markers, start_us, end_us):
    """Approximate sampler windows against timestamped driver phases, even without tracing."""
    markers = [row for row in markers if "startUnixUs" in row]
    rows = []
    for index, marker in enumerate(markers):
        stop = markers[index + 1]["startUnixUs"] if index + 1 < len(markers) else end_us
        overlap = min(end_us, stop) - max(start_us, marker["startUnixUs"])
        if overlap > 0:
            rows.append({"phase": marker["phase"], "overlapMs": overlap / 1000})
    return rows


def chrome_trace(events):
    threads = {name: index + 1 for index, name in enumerate(sorted({event["thread"] for event in events}))}
    return {"traceEvents": [{"name": event["name"], "cat": event["phase"], "ph": "X", "pid": 1,
                            "tid": threads[event["thread"]], "ts": event["startUnixUs"], "dur": event["wallNs"] / 1000,
                            "args": {key: event.get(key) for key in ["rule", "clause", "path", "runPhase", "cpuNs", "detail"]}}
                           for event in events], "displayTimeUnit": "ms"}


def summary(report):
    lines = [f"Elapsed: {report['elapsedSeconds']:.3f}s; exit: {report['exitCode']}; timeout: {report['timedOut']}"]
    if report["native"].get("parseErrors"):
        lines += [f"Warning: {len(report['native']['parseErrors'])} malformed or interleaved profiling lines; see stderr.log."]
    if "parity" in report:
        parity = report["parity"]
        lines += [f"Files: {parity['files']}; bytes: {parity['bytes']}; peak RSS: {parity['peakRssBytes']}"]
        lines += [" / ".join(f"{key}={parity[key]:.3f}ms" if parity[key] is not None else f"{key}=not measured" for key in ["compilationMs", "coldMs", "unchangedFileMs", "editedFileMs"])]
        lines += [f"Process CPU per phase (ms): {parity.get('phaseCpuMs', 'unavailable')}"]
        lines += [f"Parity: expected={parity['expected']} actual={parity['actual']} FP={len(parity['falsePositives'])} FN={len(parity['falseNegatives'])} failures={len(parity['failures'])} incremental differences={len(parity['incrementalDifferences'])}"]
    lines += ["", "Leaf span totals (worker wall sums; exclude parent spans; can overlap across threads):"]
    totals = collections.defaultdict(float)
    for row in report["native"]["spans"]:
        if row["phase"] in LEAF_PHASES:
            totals[(row["runPhase"], row["phase"])] += row["wallMs"]
    lines += [f"  {phase:10} {name:20} {ms:12.3f}ms" for (phase, name), ms in sorted(totals.items())]
    lines += ["", "Largest cold SQL stages:"]
    lines += [f"  {row['wallMs']:10.3f}ms {row['clause']} / {row['phase']} / {row['name']} / {row['path']} {dict(row['counters'])}"
              for row in [row for row in report["native"]["spans"] if row["runPhase"] == "cold" and row["phase"] in {"sql-materialize", "sql-reach-prepare", "sql-reach-walk", "sql-reach-index"}][:12]]
    for index, sample in enumerate(report["samples"], 1):
        lines += ["", f"Sample {index}: native active={sample.get('activeNativeSamples', 0)}; parked={sample.get('parkedThreadSamples', 0)}; other active={sample.get('otherActiveSamples', 0)}"]
        if sample.get("parseError"):
            lines += [f"  Parse error: {sample['parseError']}; raw capture retained in {sample['file']}"]
        lines += [f"  stack {row['percent']:6.2f}% {row['symbol']}" for row in sample.get("subsystems", [])]
        lines += [f"  self {row['percent']:6.2f}% {row['samples']:5} {row['symbol']}" for row in sample.get("self", [])[:12]]
    return "\n".join(lines) + "\n"
