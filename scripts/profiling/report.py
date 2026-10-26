"""Aggregate attributed native spans. Inclusive spans must not be added to children."""
import collections
import json
from analysis.cpu import summarize as cpu_attribution

LEAF_PHASES = {"syntax-prepare", "producer-stage", "producer-publish", "syntax-owners", "sql-release", "input-extract", "sql-input", "sql-validate", "sql-materialize", "sql-index", "sql-analyze", "sql-result", "sql-reach-prepare", "sql-reach-walk", "sql-reach-index", "anchor-fact-preparation", "anchor-lower", "anchor-prepare", "anchor-materialize", "anchor-index", "anchor-results", "anchor-schema", "anchor-release", "anchor-explain"}
PLANNING_PHASES = {"anchor-fragment", "anchor-compose", "anchor-layout", "anchor-render", "anchor-ownership"}


def sample_leaf_phases(events):
    """Use planning children when available; retain historical parent-only captures."""
    if any(event["phase"] in PLANNING_PHASES for event in events):
        return (LEAF_PHASES - {"anchor-lower"}) | PLANNING_PHASES
    return LEAF_PHASES


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
                    if event["phase"] == "anchor-working-set" or name in {"liveRelations", "peakLiveRelations"}:
                        row["counters"][name] = max(row["counters"][name], value)
                    else:
                        row["counters"][name] += value
    return {"exclusiveCpu": cpu_attribution(events), "traceEvents": len(events), "applies": applies, "parseErrors": parse_errors, "phaseMarkers": markers,
            "inputQueues": [{"runPhase": event["runPhase"], "cpuMs": (event.get("cpuNs") or 0) / 1e6,
                             "startUnixUs": event["startUnixUs"], **(event.get("detail") or {})}
                            for event in events if event["phase"] == "input-preparation"],
            "memorySpans": [{"runPhase":event["runPhase"],"phase":event["phase"],"name":event["name"],
                             "rule":event["rule"],"clause":event["clause"],"path":event["path"],
                             "startUnixUs":event["startUnixUs"],**event["memory"]}
                            for event in events if event.get("memory")],
            "inputStreams": [{"runPhase": event["runPhase"], "rule":event["rule"], "clause":event["clause"],
                              "path":event["path"], "name":event["name"], "cpuMs":(event.get("cpuNs") or 0)/1e6,
                              **(event.get("detail") or {})} for event in events if event["phase"] == "input-stream"],
            "anchorBatches": [{"runPhase": event["runPhase"], "cpuMs": (event.get("cpuNs") or 0) / 1e6,
                               **(event.get("detail") or {})}
                              for event in events if event["phase"] == "anchor-batch"],
            "anchorPreparations": [{"runPhase": event["runPhase"], "cpuMs": (event.get("cpuNs") or 0) / 1e6,
                                    **(event.get("detail") or {})}
                                   for event in events if event["phase"] == "anchor-fact-preparation"],
            "anchorStorage": [{"runPhase": event["runPhase"], **(event.get("detail") or {})}
                              for event in events if event["phase"] == "anchor-storage"],
            "anchorWorkingSets": [{"runPhase":event["runPhase"],"boundary":event["name"],
                                   "cpuMs":(event.get("cpuNs") or 0)/1e6,**(event.get("detail") or {})}
                                  for event in events if event["phase"] == "anchor-working-set"],
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


def chrome_trace(events, process_rows=()):
    threads = {name: index + 1 for index, name in enumerate(sorted({event["thread"] for event in events}))}
    return {"traceEvents": [{"name": event["name"], "cat": event["phase"], "ph": "X", "pid": 1,
                            "tid": threads[event["thread"]], "ts": event["startUnixUs"], "dur": event["wallNs"] / 1000,
                            "args": {key: event.get(key) for key in ["rule", "clause", "path", "runPhase", "cpuNs", "detail"]}}
                           for event in events] + [
        {"name": "Process tree", "ph": "C", "pid": 1, "tid": 0, "ts": row["unixUs"],
         "args": {"rootRssMiB": row["rootRssBytes"] / 1048576, "treeRssMiB": row["treeRssBytes"] / 1048576,
                  "treeCpuSecondsLowerBound": row["treeCpuSecondsLowerBound"]}} for row in process_rows], "displayTimeUnit": "ms"}


def summary(report):
    budget = report.get("sqlExecutionBudget")
    lines = [f"Elapsed: {report['elapsedSeconds']:.3f}s; exit: {report['exitCode']}; timeout: {report['timedOut']}"]
    if budget:
        if "sqliteInternal" in budget and "totalObserved" in budget:
            lines += [f"SQL execution diagnostic: {budget['status']}; {budget['observed']}/{budget['limit']} engine submissions for the complete job; {budget['sqliteInternal']} SQLite internal executions; {budget['totalObserved']} total; target exit: {report.get('targetExitCode')}. Counts do not gate performance."]
        else:
            lines += [f"Legacy SQL total: {budget['observed']}; origin split unavailable; not proof of the current engine-submission limit."]
    if "process" in report:
        process = report["process"]
        lines += [f"Sampled process tree: CPU >= {process['treeCpuSecondsLowerBound']:.3f}s; peak summed RSS {process['peakTreeRssBytes']} bytes; {process['samples']} samples.", process["note"]]
    attribution = report['native'].get('exclusiveCpu')
    if attribution:
        lines += [f"Observed span CPU: {attribution['observedCpuNs']/1e6:.3f} ms; unattributed {attribution['unattributedCpuNs']/1e6:.3f} ms. Parent and child CPU are not added."]
        lines += [f"CPU attribution coverage: missing={attribution['missingCpuEvents']}, partial overlaps={attribution['partialOverlapEvents']}, clock overdraws={attribution['cpuOverdrawEvents']}."]
        clauses = collections.Counter()
        for row in attribution['rows']:
            if row['rule'] and row['clause']:
                clauses[(row['rule'],row['clause'])] += row['selfCpuNs']
        lines += [f"  Clause CPU {cpu/1e6:.3f} ms: {rule}/{clause}" for (rule,clause),cpu in clauses.most_common(12)]
    fields = [row["fieldLookups"] for row in report["native"].get("applies", []) if row.get("fieldLookups")]
    if fields:
        lines += [f"Field navigation: {sum(row['calls'] for row in fields)} lookups; {sum(row['childrenExamined'] for row in fields)} child examinations; {sum(row['missChildrenExamined'] for row in fields)} examinations in failed lookups; maximum scan {max(row['maxScan'] for row in fields)}."]
        lines += [f"Known field edges: {sum(row.get('knownEdgeChecks', 0) for row in fields)} direct checks; {sum(row.get('knownEdgeMatches', 0) for row in fields)} matches; no sibling enumeration."]
    indexes = [row for row in report["native"].get("applies", []) if "sqlIndexWrites" in row]
    if indexes:
        writes = sum(row["sqlIndexWrites"] for row in indexes)
        nodes = sum(row["sqlIndexWrittenNodes"] for row in indexes)
        reuses = sum(row["sqlIndexReuses"] for row in indexes)
        preparation = sum(row["anchorIndexNs"] for row in indexes) / 1e9
        selection = sum(row["anchorSelectionNs"] for row in indexes) / 1e9
        lines += [f"SQL corpus index: {writes} file publications; {nodes} written nodes; {reuses} unchanged lookups; {preparation:.3f}s preparation CPU; {selection:.3f}s selection CPU."]
    batches = [row for row in report["native"].get("applies", []) if "anchorBatches" in row]
    if batches:
        lines += [f"SQL candidates: {sum(row['anchorBatches'] for row in batches)} batches; {sum(row['anchorSqlDataStatements'] for row in batches)} SQL data statements, including {sum(row['anchorSqlMaterializations'] for row in batches)} materializations; {sum(row['anchorBatchFiles'] for row in batches)} file memberships; {sum(row['anchorBatchPlans'] for row in batches)} plan memberships; {sum(row['anchorBatchFailures'] for row in batches)} failed batches; {sum(row['anchorCacheMisses'] for row in batches)} missing prepared selections; peak compact candidate storage {max(row['peakPreparedCandidateBytes'] for row in batches)} bytes."]
    anchors = [row for row in report["native"].get("spans", []) if row["phase"].startswith("anchor-") and row["phase"] not in PLANNING_PHASES | {"anchor-batch"}]
    if anchors:
        cpu = collections.Counter()
        for row in anchors:
            cpu[row["phase"]] += row["cpuMs"]
        lines += ["Candidate SQL API CPU (ms; batch parent excluded): " + "; ".join(f"{key}={value:.3f}" for key, value in sorted(cpu.items()))]
    planning = collections.Counter()
    for row in report["native"].get("spans", []):
        if row["phase"] in PLANNING_PHASES:
            planning[row["phase"]] += row["cpuMs"]
    if planning:
        lines += ["Candidate lowering breakdown (ms; included in anchor-lower): " + "; ".join(f"{key}={value:.3f}" for key, value in sorted(planning.items()))]
    for batch in report["native"].get("anchorPreparations", []) + report["native"].get("anchorBatches", []):
        reader = batch.get("reader", {})
        lines += [f"Candidate reader ({batch['runPhase']}): {sum(reader.get('reads', []))} BLOB reads, {sum(reader.get('bytes', []))} bytes, {reader.get('blob_opens', 0)} handle opens, {reader.get('texts', 0)} text extractions, {reader.get('child_steps', 0)} child steps; {batch.get('emittedRows', 0)} candidate rows."]
    working_sets = report["native"].get("anchorWorkingSets", [])
    if working_sets:
        peak = max(working_sets,key=lambda s:s["liveRelations"])
        last = working_sets[-1]
        lines += [f"Candidate SQL working set: peak sampled {peak['liveRelations']} live relations; {peak['tempReservedBytes']} temporary bytes reserved, {peak['tempFreeBytes']} free; SQLite cache {peak['sqlite']['cacheBytes']}, schema {peak['sqlite']['schemaBytes']}, statements {peak['sqlite']['statementBytes']} bytes."]
        process = last.get("process") or {}
        lines += [f"Candidate SQL final boundary: {last['liveRelations']} live relations; {last['tempFreeBytes']} temporary bytes free; process RSS {process.get('rssBytes')}, allocator in use {process.get('inUseBytes')}, allocated {process.get('allocatedBytes')} bytes. Diagnostic snapshots add overhead."]
    for queue in report["native"].get("inputQueues", []):
        memory = queue.get("ownership")
        if memory is None:
            continue
        lines += [f"Input queue ({queue['runPhase']}): {queue['jobs']} jobs; {memory['witnesses']} rows; {memory['projection']['objectEntries']} JSON fields; {memory.get('typedProjectionFields', 0)} typed fields; {memory['ownedBytesEstimate'] + memory['projection']['heapBytesEstimate'] + memory.get('typedProjectionBytes', 0)} referenced bytes estimated at preparation boundary."]
    for stream in report["native"].get("inputStreams", []):
        lines += [f"Input stream ({stream['runPhase']}, {stream['name']}): {stream['rows']} rows; peak {stream['peakBufferedRows']} buffered; {stream['columnBatches']} column batches; {stream['retainedProjectionRows']} projected rows retained after SQL import."]
    memory_spans = report["native"].get("memorySpans", [])
    growth = lambda row: (row.get("after") or {}).get("peakRssBytes", 0) - row["before"]["peakRssBytes"]
    for boundary in sorted(memory_spans, key=growth, reverse=True)[:5]:
        if growth(boundary) > 0:
            after = boundary["after"]
            lines += [f"Memory boundary {boundary['phase']}/{boundary['name']}: peak +{growth(boundary)} bytes ({after['peakRssBytes']} cumulative); allocator in use {after.get('inUseBytes')}, allocated {after.get('allocatedBytes')} bytes. Inclusive boundaries overlap."]
    for snapshot in report.get("memorySnapshots", []):
        lines += [f"Memory snapshot: {snapshot['file']}; exit={snapshot.get('exitCode')}; error={snapshot.get('error')}"]
        memory = snapshot.get("summary", {})
        if "physicalFootprintBytesApprox" in memory:
            lines += [f"  Approximate physical footprint: {memory['physicalFootprintBytesApprox']} bytes; region and allocator totals are separate views."]
    if report["native"].get("parseErrors"):
        lines += [f"Warning: {len(report['native']['parseErrors'])} malformed or interleaved profiling lines; see stderr.log."]
    if "parity" in report:
        parity = report["parity"]
        if parity.get("format") == "rerail.data-layer-parity/v1":
            lines += [f"Source replay: calls={parity['replayedCalls']} native={parity['nativeCalls']} host guards={parity['hostGuardCalls']} positive={parity['positiveCalls']} clean={parity['cleanCalls']}"]
            lines += [f"Parity: FP={parity['falsePositives']} FN={parity['falseNegatives']} failures={parity['engineFailures']} execution differences={parity['executionPathDifferences']} missing coverage={len(parity['missingCoverage'])}"]
        elif parity.get("format") == "rerail.data-layer-incremental-parity/v1":
            lines += [f"Incremental replay: checks={parity['checks']} transitions={parity['transitions']} differences={parity['differences']}"]
        else:
            lines += [f"Files: {parity['files']}; bytes: {parity['bytes']}; peak RSS: {parity['peakRssBytes']}"]
            lines += [" / ".join(f"{key}={parity[key]:.3f}ms" if parity[key] is not None else f"{key}=not measured" for key in ["compilationMs", "coldMs", "unchangedFileMs", "editedFileMs"])]
            lines += [f"Process CPU per phase (ms): {parity.get('phaseCpuMs', 'unavailable')}"]
            for phase in ["file-inputs", "global-results"]:
                rows = [row for row in parity.get("applies", []) if row["phase"] == phase]
                if rows:
                    lines += [f"  {phase}: {sum(row['cpuMs'] for row in rows):.3f}ms CPU across {len(rows)} applies; peak boundary RSS {max(row['rssBytes'] for row in rows)} bytes."]
            for checkpoint in parity.get("memoryCheckpoints", []):
                ownership = checkpoint["workspace"].get("ownership", {})
                lines += [f"  Census {checkpoint['phase']}: {ownership.get('witnesses')} witness references; {ownership.get('uniqueProjections')} unique projections; {ownership.get('projection', {}).get('objectEntries')} JSON fields; census CPU {checkpoint['censusCpuMs']:.3f}ms."]
            lines += [f"Parity: expected={parity['expected']} actual={parity['actual']} FP={len(parity['falsePositives'])} FN={len(parity['falseNegatives'])} failures={len(parity['failures'])} incremental differences={len(parity['incrementalDifferences'])}"]
    lines += ["", "Leaf span totals (worker wall sums; exclude parent spans; can overlap across threads):"]
    totals = collections.defaultdict(float)
    for row in report["native"]["spans"]:
        if row["phase"] in LEAF_PHASES:
            totals[(row["runPhase"], row["phase"])] += row["wallMs"]
    lines += [f"  {phase:10} {name:20} {ms:12.3f}ms" for (phase, name), ms in sorted(totals.items())]
    lines += ["", "Largest cold SQL stages:"]
    lines += [f"  {row['wallMs']:10.3f}ms {row['clause']} / {row['phase']} / {row['name']} / {row['path']} {dict(row['counters'])}"
              for row in [row for row in report["native"]["spans"] if row["runPhase"] == "cold" and row["phase"] in {"sql-materialize", "sql-reach-prepare", "sql-reach-walk", "sql-reach-index", "anchor-lower", "anchor-prepare", "anchor-materialize", "anchor-index", "anchor-results", "anchor-schema", "anchor-release", "anchor-explain"}][:12]]
    for index, sample in enumerate(report["samples"], 1):
        lines += ["", f"Sample {index}: native active={sample.get('activeNativeSamples', 0)}; parked={sample.get('parkedThreadSamples', 0)}; other active={sample.get('otherActiveSamples', 0)}"]
        lines += [sample.get("samplingBasis", "Wall-time stack observations, rather than CPU-time shares.")]
        if "ioSyscallSamples" in sample:
            lines += [f"  I/O syscall observations: {sample['ioSyscallSamples']}; frames can include execution or blocked I/O."]
        if sample.get("parseError"):
            lines += [f"  Parse error: {sample['parseError']}; raw capture retained in {sample['file']}"]
        lines += [f"  stack {row['percent']:6.2f}% {row['symbol']}" for row in sample.get("subsystems", [])]
        lines += [f"  self {row['percent']:6.2f}% {row['samples']:5} {row['symbol']}" for row in sample.get("self", [])[:12]]
    return "\n".join(lines) + "\n"
