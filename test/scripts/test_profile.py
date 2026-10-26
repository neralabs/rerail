import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

PROFILING = Path(__file__).resolve().parents[2] / "scripts" / "profiling"
sys.path.insert(0, str(PROFILING))

from report import aggregate, chrome_trace, phase_overlap, summary
from run import recording_from_source, select_recording
from sample import bottom_up
from compare import compare
from process import cpu_seconds, parse_ps, ProcessMonitor, vmmap_summary
from oracle import compare_snapshot, workload_digest
from unittest.mock import patch


class ProfileTests(unittest.TestCase):
    def test_complete_memory_gate_uses_os_peak_even_when_close_exceeds_the_driver_sample(self):
        from workloads.full_repository import measurement_result
        parity = dict(files=9021, bytes=137924324, expected=24993, actual=24993,
                      compilationMs=1820, coldMs=2095695, phaseCpuMs={'cold':{'userMs':1302995,'systemMs':591335}},
                      peakRssBytes=6571507712, falsePositives=[], falseNegatives=[], failures=[])
        resource = {'peakRssBytes':7372029952}
        close = {'after':{'peakRssBytes':7372029952}}
        result = measurement_result(parity, resource, close)
        self.assertEqual(result['peakRssBytes'],7372029952)
        self.assertEqual(result['driverPeakRssBytes'],6571507712)
        self.assertEqual(result['close'],close)
        self.assertEqual(parity['peakRssBytes'],6571507712)
        self.assertNotIn('close',measurement_result(parity,resource))

    def test_external_profile_targets_workload_child_instead_of_time_wrapper(self):
        monitor=ProcessMonitor(10,Path('/unused'))
        monitor.rows=[{'processes':[
            dict(pid=10,parent=1,rssBytes=512*1024,cpuSeconds=0,command='/usr/bin/time'),
            dict(pid=11,parent=10,rssBytes=4*2**30,cpuSeconds=100,command='node')]}]
        self.assertEqual(monitor.profile_pid(),11)
        monitor.rows=[]
        self.assertEqual(monitor.profile_pid(),10)

    def test_allocator_census_requires_explicit_memory_request(self):
        from run import instrumentation_env
        from types import SimpleNamespace
        args=SimpleNamespace(sql_profile=False,query_plans=False,trace='on',memory_census=False)
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {'RERAIL_PROFILE_MEMORY_CENSUS':'1'}):
            self.assertIsNone(instrumentation_env(args,Path(directory)).get('RERAIL_PROFILE_MEMORY_CENSUS'))
            args.memory_census=True
            self.assertEqual(instrumentation_env(args,Path(directory))['RERAIL_PROFILE_MEMORY_CENSUS'],'1')

    def test_cpu_attribution_subtracts_children_and_keeps_threads_separate(self):
        from analysis.cpu import summarize
        def event(start,wall,cpu,thread='one',rule='',clause='',phase='stage'):
            return dict(startUnixUs=start,wallNs=wall*1000,cpuNs=cpu,thread=thread,
                        rule=rule,clause=clause,phase=phase,name=phase,runPhase='cold')
        events=[event(0,100,100,rule='r',clause='c',phase='parent'),
                event(10,60,60,phase='child'),event(20,20,20,phase='grandchild'),
                event(0,100,20,thread='two',phase='other')]
        result=summarize(events)
        self.assertEqual(result['observedCpuNs'],120)
        self.assertEqual(result['unattributedCpuNs'],20)
        self.assertEqual(sum(r['selfCpuNs'] for r in result['rows']),120)
        self.assertEqual({r['name']:r['selfCpuNs'] for r in result['rows']},
                         {'parent':40,'child':40,'grandchild':20,'other':20})
        self.assertTrue(all(r['clause']=='c' for r in result['rows'] if r['name']!='other'))
        missing=summarize([event(0,10,None)])
        self.assertEqual(missing['missingCpuEvents'],1)
        self.assertEqual(missing['observedCpuNs'],0)

    def test_lowering_breakdown_keeps_parent_and_children_separate(self):
        import report as profiling_report
        events = [{"rule":"", "clause":"", "path":None, "name":"planning", "thread":"main",
                   "startUnixUs":0, "detail":{}, "phase":phase, "wallNs":ms*1_000_000, "cpuNs":ms*1_000_000}
                  for phase, ms in [("anchor-lower",10),("anchor-fragment",4),("anchor-ownership",6)]]
        native, parsed = aggregate("\n".join("RERAIL_TRACE "+json.dumps(event) for event in events))
        text = summary({"elapsedSeconds":1,"exitCode":0,"timedOut":False,"native":native,"samples":[]})
        self.assertIn("Candidate lowering breakdown (ms; included in anchor-lower): anchor-fragment=4.000; anchor-ownership=6.000", text)
        api = next(line for line in text.splitlines() if line.startswith("Candidate SQL API CPU"))
        self.assertIn("anchor-lower=10.000", api)
        self.assertNotIn("anchor-fragment", api)
        leaves = profiling_report.sample_leaf_phases(parsed)
        self.assertNotIn("anchor-lower", leaves)
        self.assertIn("anchor-fragment", leaves)
        self.assertIn("anchor-lower", profiling_report.sample_leaf_phases(parsed[:1]))

    def test_vmmap_keeps_residency_and_allocator_views_separate(self):
        result = vmmap_summary("""Physical footprint:         1.5G
Physical footprint (peak):  2G
MALLOC_SMALL                       2G     1G     512M     1G       0K       0K       0K        2
MALLOC_SMALL (empty)              128M     64M     32M       0K       0K       0K       0K        1
MALLOC ZONE                         SIZE       SIZE       SIZE       SIZE      COUNT  ALLOCATED
DefaultMallocZone_0x123              2G       1G       512M       1G   1234       1.5G         0K      0%    2
""")
        self.assertTrue(result["recognized"])
        self.assertTrue(result["roundedValues"])
        self.assertEqual(result["physicalFootprintBytesApprox"], int(1.5 * 2**30))
        self.assertEqual(result["peakPhysicalFootprintBytesApprox"], 2 * 2**30)
        self.assertEqual(result["regions"]["MALLOC_SMALL"]["swappedBytesApprox"], 2**30)
        self.assertEqual(result["regions"]["MALLOC_SMALL (empty)"]["residentBytesApprox"], 64 * 2**20)
        self.assertEqual(result["allocatorZones"][0]["allocationCount"], 1234)
        self.assertFalse(vmmap_summary("vmmap could not inspect this process")["recognized"])

    def test_summary_accepts_source_replay_counts(self):
        native, _ = aggregate("")
        report = {"elapsedSeconds": 1, "exitCode": 0, "timedOut": False, "native": native, "samples": [],
                  "parity": {"format": "rerail.data-layer-parity/v1", "replayedCalls": 4,
                             "nativeCalls": 3, "hostGuardCalls": 1, "positiveCalls": 2, "cleanCalls": 2,
                             "falsePositives": 0, "falseNegatives": 0, "engineFailures": 0,
                             "executionPathDifferences": 0, "missingCoverage": []}}
        text = summary(report)
        self.assertIn("Source replay: calls=4 native=3 host guards=1 positive=2 clean=2", text)
        self.assertIn("FP=0 FN=0 failures=0 execution differences=0 missing coverage=0", text)
        report.update(exitCode=1, targetExitCode=0,
                      sqlExecutionBudget={"status": "budget-exceeded", "observed": 101, "limit": 100, "sqliteInternal": 500, "totalObserved": 601})
        self.assertIn("101/100 engine submissions for the complete job", summary(report))
        self.assertIn("500 SQLite internal executions; 601 total", summary(report))
        self.assertIn("target exit: 0", summary(report))
        report["sqlExecutionBudget"] = {"status": "budget-exceeded", "observed": 101, "limit": 100}
        self.assertIn("Legacy SQL total: 101; origin split unavailable", summary(report))

    def test_summary_accepts_incremental_replay(self):
        native, _ = aggregate("")
        report = {"elapsedSeconds": 1, "exitCode": 0, "timedOut": False, "native": native, "samples": [],
                  "parity": {"format": "rerail.data-layer-incremental-parity/v1",
                             "checks": 10, "transitions": 40, "differences": 0}}
        self.assertIn("Incremental replay: checks=10 transitions=40 differences=0", summary(report))

    def test_candidate_working_set_keeps_gauges_and_live_storage_separate(self):
        events=[]
        for count,free in [(100,0),(20,800),(0,1000)]:
            events.append("RERAIL_TRACE "+json.dumps({"rule":"r","clause":"c","path":None,"phase":"anchor-working-set","name":"progress",
                "wallNs":1000,"cpuNs":500,"startUnixUs":0,"thread":"main","detail":{
                    "liveRelations":count,"tempReservedBytes":1000,"tempFreeBytes":free,
                    "sqlite":{"cacheBytes":50,"schemaBytes":10,"statementBytes":0},
                    "process":{"rssBytes":500,"inUseBytes":100,"allocatedBytes":400}}}))
        native,_=aggregate("\n".join(events))
        self.assertEqual(len(native["anchorWorkingSets"]),3)
        self.assertEqual(native["spans"][0]["counters"]["liveRelations"],100)
        self.assertEqual(native["spans"][0]["counters"]["tempReservedBytes"],1000)
        text=summary({"elapsedSeconds":1,"exitCode":0,"timedOut":False,"native":native,"samples":[]})
        self.assertIn("peak sampled 100 live relations",text)
        self.assertIn("0 live relations; 1000 temporary bytes free",text)

    def test_memory_spans_preserve_boundaries_without_summing_high_water_marks(self):
        event={"rule":"r","clause":"c","path":"a.py","phase":"sql-materialize","name":"facts",
               "wallNs":1000,"cpuNs":500,"startUnixUs":0,"thread":"main",
               "memory":{"before":{"peakRssBytes":100,"allocatedBytes":80,"inUseBytes":60},
                         "after":{"peakRssBytes":150,"allocatedBytes":90,"inUseBytes":70}}}
        native,_=aggregate("RERAIL_TRACE "+json.dumps(event))
        self.assertEqual(native["memorySpans"][0]["before"]["peakRssBytes"],100)
        self.assertEqual(native["memorySpans"][0]["after"]["peakRssBytes"],150)
        self.assertNotIn("peakRssBytes",native["spans"][0]["counters"])
        self.assertIn("peak +50 bytes (150 cumulative)",summary({"elapsedSeconds":1,"exitCode":0,"timedOut":False,"native":native,"samples":[]}))

    def test_stream_buffer_uses_maximum_per_stream_not_sum_of_row_counts(self):
        event = {"rule":"r","clause":"c","path":"a.py","phase":"input-stream","name":"facts",
                 "wallNs":1000,"cpuNs":500,"startUnixUs":0,"thread":"main",
                 "detail":{"rows":600,"peakBufferedRows":256,"columnBatches":3,"retainedProjectionRows":0}}
        native, _ = aggregate("RERAIL_TRACE " + json.dumps(event))
        self.assertEqual(native["inputStreams"][0]["peakBufferedRows"],256)
        text = summary({"elapsedSeconds":1,"exitCode":0,"timedOut":False,"native":native,"samples":[]})
        self.assertIn("600 rows; peak 256 buffered; 3 column batches; 0 projected rows retained",text)

    def test_typed_input_census_reports_column_bytes_without_inventing_json_fields(self):
        event = {"rule":"r", "clause":"c", "path":"a.py", "phase":"input-preparation",
                 "name":"admitted-relational-inputs", "wallNs":1000, "cpuNs":500,
                 "startUnixUs":0, "thread":"main", "detail":{"jobs":1, "ownership":{
                     "witnesses":2, "ownedBytesEstimate":24, "typedProjectionFields":6,
                     "typedProjectionBytes":100, "projection":{"objectEntries":0,"heapBytesEstimate":0}}}}
        native, _ = aggregate("RERAIL_TRACE " + json.dumps(event))
        text = summary({"elapsedSeconds":1,"exitCode":0,"timedOut":False,"native":native,"samples":[]})
        self.assertIn("0 JSON fields; 6 typed fields; 124 referenced bytes", text)
        self.assertEqual(native["inputQueues"][0]["ownership"]["typedProjectionFields"],6)

    def test_cross_runtime_comparison_preserves_missing_files_and_duplicate_findings(self):
        from copy import deepcopy
        rows = [{"path": "a.py", "source": "pass"}]
        recording = {"commit": "pin", "files": rows, "events": [{"check": "required", "findings": []}]}
        key = '["data-layer-required","absent.py",1,"missing"]'
        profile = {"exitCode": 1, "timedOut": False, "parity": {"commit": "pin", "expected": 0, "actual": 2,
                   "falsePositives": [key, key], "falseNegatives": [], "failures": [], "incrementalDifferences": [],
                   "phaseCpuMs": {"cold": {"userMs": 20, "systemMs": 1}}, "peakRssBytes": 2000, "nativeSha256": "binary"}}
        profile["parity"]["workloadSha256"] = workload_digest(recording)
        oracle = {"commit": "pin", "source_mismatches": [], "files": rows, "workers": 1, "oracle_cpu_ms": 3,
                  "peak_rss_bytes": 1000, "events": [{"check": "required", "findings": [{"path": "absent.py", "line": 1, "code": "missing"}] * 2}]}
        result = compare_snapshot(profile, recording, oracle)
        self.assertTrue(result["passed"])
        self.assertEqual(result["nativeToPythonCpuRatio"], 7)
        different = deepcopy(oracle)
        different["files"] = [{"path": "a.py", "source": "different"}]
        with self.assertRaisesRegex(ValueError, "Source files differ"): compare_snapshot(profile, recording, different)
        different = deepcopy(oracle)
        different["events"][0]["findings"].pop()
        self.assertEqual(len(compare_snapshot(profile, recording, different)["falsePositives"]), 1)

    def test_process_cpu_and_rss_units(self):
        self.assertEqual(cpu_seconds("2-03:04:05.50"), 183845.5)
        self.assertAlmostEqual(cpu_seconds("01:02.03"), 62.03)
        rows = parse_ps(" 10 1 2048 4096 00:01.50 /path with spaces/node\n")
        self.assertEqual(rows[10]["rssBytes"], 2097152)
        self.assertEqual(rows[10]["command"], "/path with spaces/node")

    def test_chrome_trace_adds_memory_counters_at_process_sample_times(self):
        result = chrome_trace([], [{"unixUs": 123, "rootRssBytes": 1048576, "treeRssBytes": 2097152,
                                    "treeCpuSecondsLowerBound": 2.5}])
        event = result["traceEvents"][0]
        self.assertEqual((event["ts"], event["ph"], event["args"]["treeRssMiB"]), (123, "C", 2))

    def test_process_tree_keeps_exited_worker_cpu_without_counting_unrelated_tasks(self):
        from types import SimpleNamespace
        snapshots = ["10 1 100 200 00:01 node\n12 11 30 40 00:03 child\n11 10 20 30 00:02 worker\n99 1 90 100 00:50 other\n",
                     "10 1 110 200 00:02 node\n"]
        with tempfile.TemporaryDirectory() as directory, patch("process.subprocess.run", side_effect=[SimpleNamespace(stdout=row) for row in snapshots]):
            monitor = ProcessMonitor(10, Path(directory) / "process.jsonl")
            monitor.sample(10)
            monitor.sample(10)
            report = monitor.report()
        self.assertEqual(report["treeCpuSecondsLowerBound"], 7)
        self.assertEqual(report["peakTreeRssBytes"], 150 * 1024)
        self.assertEqual(report["processesSeen"], 3)

    def test_comparison_rejects_a_different_binary_or_failed_parity(self):
        import copy
        parity = {"commit":"pin", "files":1, "bytes":10, "workloadSha256":"workload", "nativeSha256":"binary", "policyDigests":{}, "allocator":"system",
                  "falsePositives":[], "falseNegatives":[], "failures":[], "incrementalDifferences":[],
                  "coldMs":20, "unchangedFileMs":None, "editedFileMs":None, "peakRssBytes":100}
        before = {"parity":parity, "exitCode":0, "timedOut":False, "native":{"spans":[]}}
        after = copy.deepcopy(before); after["parity"]["coldMs"] = 10
        self.assertEqual(compare(before, after)["phases"]["cold"]["wallSpeedup"], 2)
        after["nativeInput"] = "captures"
        self.assertEqual(compare(before, after)["changedSettings"]["nativeInput"]["experiment"], "captures")
        after["parity"]["nativeSha256"] = "different"
        with self.assertRaisesRegex(ValueError, "nativeSha256"): compare(before, after)
        self.assertEqual(compare(before, after, allow_engine_changes=True)["changedNative"], {"baseline": "binary", "experiment": "different"})
        after["parity"]["workloadSha256"] = "different-workload"
        with self.assertRaisesRegex(ValueError, "workloadSha256"): compare(before, after, allow_engine_changes=True)
        after = copy.deepcopy(before); after["trace"] = "off"
        with self.assertRaisesRegex(ValueError, "modes do not match"): compare(before, after)
        after = copy.deepcopy(before); after["sampleModules"] = ["*"]
        with self.assertRaisesRegex(ValueError, "sampleModules"): compare(before, after)
        after = copy.deepcopy(before); after["sqlProfile"] = True
        with self.assertRaisesRegex(ValueError, "sqlProfile"): compare(before, after)
        after = copy.deepcopy(before); after["parity"]["falseNegatives"] = ["miss"]
        with self.assertRaisesRegex(ValueError, "passing parity"): compare(before, after)
        after = copy.deepcopy(before)
        before["parity"]["policyDigests"] = {"rule": "old"}
        after["parity"]["policyDigests"] = {"rule": "sql-rewrite"}
        with self.assertRaisesRegex(ValueError, "policyDigests"): compare(before, after)
        self.assertEqual(compare(before, after, True)["changedPolicies"], {"rule": {"baseline": "old", "experiment": "sql-rewrite"}})

    def test_bottom_up_residuals_recursion_and_parked_workers(self):
        result = bottom_up("""Call graph:
    10 Thread_1
    + 10 root (in rerail_engine.node) + 1
    +   8 recurse (in rerail_engine.node) + 1
    +     5 recurse (in rerail_engine.node) + 2
    +       5 compare (in libc) + 3
    10 Thread_2
      10 root (in rerail_engine.node) + 1
        10 __psynch_cvwait (in kernel) + 2
Total number in stack (recursive counted multiple, when >=5):
""")
        self.assertEqual(result["activeNativeSamples"], 10)
        self.assertEqual(result["parkedThreadSamples"], 10)
        self.assertEqual(result["nativeThreadSamples"], {"Thread_1": 10})
        self.assertEqual(sum(row["samples"] for row in result["self"]), 10)
        self.assertEqual(result["self"][0]["symbol"], "compare (libc)")
        self.assertEqual(next(row["samples"] for row in result["inclusive"] if row["symbol"].startswith("recurse")), 8)
        self.assertEqual(result["bottomUpStacks"][0]["frames"][:2], ["compare (libc)", "recurse (rerail_engine.node)"])

    def test_native_spans_keep_phase_path_and_nullable_cpu(self):
        event = {"rule": "rule", "clause": "clause", "path": "a.py", "phase": "sql-materialize", "name": "names",
                 "wallNs": 3000000, "cpuNs": None, "detail": {"vmSteps": 50}}
        log = 'RERAIL_PROFILE_PHASE {"phase":"cold"}\n' + 'RERAIL_TRACE ' + json.dumps(event) + '\n'
        summary, events = aggregate(log + log)
        self.assertEqual(summary["traceEvents"], 2)
        row = summary["spans"][0]
        self.assertEqual((row["runPhase"], row["path"], row["calls"], row["wallMs"], row["cpuMeasuredCalls"]), ("cold", "a.py", 2, 6, 0))
        self.assertEqual(row["counters"]["vmSteps"], 100)
        self.assertEqual(len(events), 2)

    def test_bottom_up_marks_selected_io_without_claiming_cpu_time(self):
        result = bottom_up("""Call graph:
    8 Thread_1
    + 8 sqlite3_step (in rerail_engine.node) + 1
    +   8 pwrite (in libsystem_kernel.dylib) + 2
    4 Thread_2
    + 4 unrelated (in other.node) + 1
    +   4 pread (in libsystem_kernel.dylib) + 2
    10 Thread_3
    + 10 root (in rerail_engine.node) + 1
    +   10 __psynch_cvwait (in kernel) + 2
Total number in stack (recursive counted multiple, when >=5):
""")
        self.assertEqual(result["activeNativeSamples"], 8)
        self.assertEqual(result["ioSyscallSamples"], 8)
        self.assertEqual(result["ioSyscallSymbols"], {"pwrite (libsystem_kernel.dylib)": 8})
        self.assertEqual(result["otherActiveSamples"], 4)
        self.assertEqual(result["parkedThreadSamples"], 10)
        self.assertIn("not CPU-time shares", result["samplingBasis"])

    def test_bottom_up_can_profile_standalone_sql_and_reachability(self):
        source = """Call graph:
    10 Thread_1
    + 10 Python (in Python) + 1
    +   8 sqlite3VdbeExec (in libsqlite3.dylib) + 1
    +     3 rerail_reachability_spike::graph::Graph::reachable (in reach.dylib) + 1
    +     5 malloc (in libc) + 1
    +   2 allocate (in Python) + 1
Total number in stack (recursive counted multiple, when >=5):
"""
        self.assertEqual(bottom_up(source)["activeNativeSamples"],0)
        result = bottom_up(source,["libsqlite3"])
        self.assertEqual(result["activeNativeSamples"],8)
        self.assertEqual(result["subsystems"], [{"symbol":"sqlite","samples":5,"percent":62.5},
                                                {"symbol":"reachability-graph","samples":3,"percent":37.5}])
        self.assertEqual(bottom_up(source,["*"])["activeNativeSamples"],10)

    def test_bottom_up_attributes_sqlite_allocator_leaves_once(self):
        result = bottom_up("""Call graph:
    10 Thread_1
    + 10 root (in rerail_engine.node) + 1
    +   8 sqlite3VdbeExec (in rerail_engine.node) + 1
    +     8 malloc (in libsystem_malloc.dylib) + 1
    +   2 matcher (in rerail_engine.node) + 1
Total number in stack (recursive counted multiple, when >=5):
""")
        self.assertEqual(result["subsystems"], [{"symbol": "sqlite", "samples": 8, "percent": 80.0},
                                                {"symbol": "other-native", "samples": 2, "percent": 20.0}])

    def test_unsymbolicated_frames_preserve_stack_depth(self):
        result = bottom_up("""Call graph:
    3 Thread_1
    + 3 root (in rerail_engine.node) + 1
    +   2 search (in rerail_engine.node) + 1
    +     2 compare (in libc) + 1
    +   1 0x0
    +     1 search (in rerail_engine.node) + 2
Total number in stack (recursive counted multiple, when >=5):
""")
        self.assertEqual(result["activeNativeSamples"], 3)
        self.assertEqual(sum(row["samples"] for row in result["self"]), 3)
        self.assertTrue(any("0x0 (unknown)" in row["frames"] for row in result["bottomUpStacks"]))

    def test_sampler_phase_overlap_without_native_tracing(self):
        markers = [{"phase": "cold", "startUnixUs": 1000}, {"phase": "edit", "startUnixUs": 5000}]
        self.assertEqual(phase_overlap(markers, 4000, 7000), [{"phase": "cold", "overlapMs": 1}, {"phase": "edit", "overlapMs": 2}])
        self.assertEqual(phase_overlap(markers, 0, 500), [])

    def test_partial_and_interleaved_logs_preserve_valid_evidence(self):
        summary, events = aggregate('RERAIL_TRACE {"wallNs":\nRERAIL_CODE_RULE_PROFILE {"revision":1}\n')
        self.assertEqual(summary["applies"], [{"phase": "startup", "revision": 1}])
        self.assertEqual(summary["parseErrors"][0]["line"], 1)
        self.assertEqual(events, [])

    def test_source_selection_and_timeline_keep_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "a.py").write_text("pass")
            selection = {"commit": "pin", "checks": ["check"], "expectations": {"check": [{"path": "a.py", "line": 1}]},
                         "files": [{"path": "a.py", "sha256": hashlib.sha256(b"pass").hexdigest()}]}
            recording = recording_from_source(root, selection)
            self.assertEqual(recording["events"][0]["findings"], selection["expectations"]["check"])
        event = {"thread": "worker", "name": "nodes", "phase": "sql-materialize", "startUnixUs": 100,
                 "wallNs": 2000, "rule": "rule", "path": "a.py"}
        row = chrome_trace([event])["traceEvents"][0]
        self.assertEqual((row["ph"], row["ts"], row["dur"], row["args"]["path"]), ("X", 100, 2, "a.py"))

    def test_subset_checks_source_and_keeps_positive_findings(self):
        recording = {"commit": "pin", "source_mismatches": [], "oracle_ms": 10,
                     "files": [{"path": "a.py", "source": "pass"}, {"path": "b.py", "source": ""}],
                     "events": [{"check": "check", "findings": [{"path": "a.py", "line": 1}, {"path": "b.py", "line": 2}]}]}
        selection = {"commit": "pin", "checks": ["check"], "files": [{"path": "a.py", "sha256": hashlib.sha256(b"pass").hexdigest()}]}
        result = select_recording(recording, selection)
        self.assertEqual(result["events"][0]["findings"], [{"path": "a.py", "line": 1}])
        self.assertIsNone(result["oracle_ms"])
        selection["files"][0]["sha256"] = "wrong"
        with self.assertRaisesRegex(ValueError, "hash mismatch"):
            select_recording(recording, selection)

    def test_snapshot_expectations_keep_missing_file_diagnostics(self):
        recording = {"commit": "pin", "source_mismatches": [], "files": [{"path": "a.py", "source": "pass"}],
                     "events": [{"check": "required", "findings": []}]}
        expected = [{"path": "missing.py", "line": 1, "code": "required-file"}]
        selection = {"commit": "pin", "checks": ["required"],
                     "files": [{"path": "a.py", "sha256": hashlib.sha256(b"pass").hexdigest()}],
                     "expectations": {"required": expected}}
        self.assertEqual(select_recording(recording, selection)["events"][0]["findings"], expected)

    def test_deadline_kills_worker_and_retains_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "run"
            result = subprocess.run([sys.executable, str(PROFILING / "run.py"), "--output", str(output),
                                     "--deadline", "5", "--trace", "off", "--", sys.executable, "-c", "import time;time.sleep(20)"],
                                    capture_output=True, timeout=8)
            self.assertEqual(result.returncode, 124, result.stderr)
            self.assertTrue(json.loads((output / "profile.json").read_text())["timedOut"])

    def test_statement_only_capture_controls_replay_without_losing_sql_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "run"
            result = subprocess.run([sys.executable, str(PROFILING / "run.py"), "--output", str(output),
                "--deadline", "5", "--trace", "off", "--sql-profile", "--sql-trace-only", "--", sys.executable,
                "-c", "import json,os;print(json.dumps([os.getenv('RERAIL_SQL_PROFILE_REPLAY'), bool(os.getenv('RERAIL_SQL_PROFILE_DIR'))]))"],
                capture_output=True, timeout=8)
            self.assertEqual(result.returncode, 1, result.stderr)  # The Python target has no SQL instrumentation.
            self.assertEqual(json.loads((output / "stdout.log").read_text()), ["0", True])
            self.assertTrue(json.loads((output / "profile.json").read_text())["sqlTraceOnly"])

    def test_sql_capture_requires_instrumented_execution_and_clears_inherited_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for enabled in [False, True]:
                output = root / str(enabled)
                result = subprocess.run([sys.executable, str(PROFILING / "run.py"), "--output", str(output),
                    "--deadline", "5", "--trace", "off", *(["--sql-profile"] if enabled else []), "--", sys.executable,
                    "-c", "import json,os;print(json.dumps(os.getenv('RERAIL_SQL_PROFILE_DIR')))"],
                    env={**os.environ, "RERAIL_SQL_PROFILE_DIR": str(root / "inherited")}, capture_output=True, timeout=8)
                self.assertEqual(result.returncode, 1 if enabled else 0, result.stderr)
                self.assertEqual(json.loads((output / "stdout.log").read_text()), str((output / "sql" / "raw").resolve()) if enabled else None)
                report = json.loads((output / "profile.json").read_text())
                self.assertEqual(report["sqlProfile"], enabled)
                if enabled:
                    self.assertIn("sqlProfileError", report)
                    self.assertIn("No SQL sessions", (output / "sql-report.log").read_text())


if __name__ == "__main__":
    unittest.main()
