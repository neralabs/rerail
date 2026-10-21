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

from report import aggregate, chrome_trace, phase_overlap
from run import recording_from_source, select_recording
from sample import bottom_up
from compare import compare


class ProfileTests(unittest.TestCase):
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

    def test_deadline_kills_worker_and_retains_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "run"
            result = subprocess.run([sys.executable, str(PROFILING / "run.py"), "--output", str(output),
                                     "--deadline", "5", "--trace", "off", "--", sys.executable, "-c", "import time;time.sleep(20)"],
                                    capture_output=True, timeout=8)
            self.assertEqual(result.returncode, 124, result.stderr)
            self.assertTrue(json.loads((output / "profile.json").read_text())["timedOut"])

    def test_experimental_flags_are_explicit_and_recorded(self):
        with tempfile.TemporaryDirectory() as temporary:
            for name, flags, expected in [("default", [], ["32", None]),
                                          ("control", ["--insert-batch", "1"], ["1", None]),
                                          ("experiment", ["--insert-batch", "7", "--shared-input-index"], ["7", "1"])]:
                output = Path(temporary) / name
                result = subprocess.run([sys.executable, str(PROFILING / "run.py"), "--output", str(output),
                                         "--deadline", "5", "--trace", "off", *flags, "--", sys.executable, "-c",
                                         "import json,os;print(json.dumps([os.getenv('RERAIL_SQL_INSERT_BATCH'),os.getenv('RERAIL_SHARED_INPUT_INDEX')]))"],
                                        env={**os.environ, "RERAIL_SQL_INSERT_BATCH": "256", "RERAIL_SHARED_INPUT_INDEX": "1"},
                                        capture_output=True, timeout=8)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(json.loads((output / "stdout.log").read_text()), expected)
                report = json.loads((output / "profile.json").read_text())
                self.assertEqual([str(report["insertBatch"]), "1" if report["sharedInputIndex"] else None], expected)


if __name__ == "__main__":
    unittest.main()
