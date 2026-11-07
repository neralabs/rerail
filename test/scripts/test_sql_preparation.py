"""Exercise production vocabulary SQL, including empty results and rollback."""
import importlib.util
import json
from pathlib import Path
import re
import sqlite3
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SQL = ROOT / "native/src/code_rule_dsl/matcher/anchors/sql"


class VocabularyTests(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(":memory:", isolation_level=None)
        self.db.execute("PRAGMA foreign_keys=ON")
        self.db.execute("CREATE TABLE vlex(lang INTEGER,term TEXT,n INTEGER,PRIMARY KEY(lang,term)) WITHOUT ROWID")
        self.db.executescript((SQL / "vocabulary.sql").read_text())
        self.calls = []
        self.db.create_function("nera_term_matches", 2, self.matches)
        self.db.create_function("nera_term_matches", 3, lambda expression, term, _: self.matches(expression, term))

    def tearDown(self):
        self.db.close()

    def matches(self, expression, term):
        self.calls.append((expression, term))
        return re.search(expression, term) is not None

    def prepare(self):
        self.db.execute((SQL / "vocabulary-fill.sql").read_text())
        self.db.execute("UPDATE vocabulary_predicate SET ready=1 WHERE ready=0")
        self.db.execute("DELETE FROM vocabulary_pending")

    def test_new_predicates_terms_empty_matches_and_last_occurrence(self):
        self.db.executemany("INSERT INTO vlex VALUES(1,?,1)", [("target",), ("other",)])
        self.db.executemany("INSERT INTO vocabulary_predicate VALUES(1,?,0)", [("^target$",), ("^absent$",)])
        self.prepare()
        self.assertEqual(len(self.calls), 4)
        self.assertEqual(self.db.execute("SELECT term FROM vocabulary_term").fetchall(), [("target",)])
        self.calls.clear()
        self.prepare()
        self.assertEqual(self.calls, [])
        self.db.execute("UPDATE vlex SET n=n+1 WHERE term='target'")
        self.prepare()
        self.assertEqual(self.calls, [])
        self.db.execute("INSERT INTO vlex VALUES(1,'new',1)")
        self.prepare()
        self.assertEqual(len(self.calls), 2)
        self.db.execute("DELETE FROM vlex WHERE term='target'")
        self.assertEqual(self.db.execute("SELECT term FROM vocabulary_term").fetchall(), [])
        self.calls.clear()
        self.db.execute("INSERT INTO vlex VALUES(1,'target',1)")
        self.prepare()
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.db.execute("SELECT term FROM vocabulary_term").fetchall(), [("target",)])

    def test_failed_publication_rolls_back_matches_and_completion(self):
        self.db.execute("INSERT INTO vlex VALUES(1,'target',1)")
        self.db.execute("INSERT INTO vocabulary_predicate VALUES(1,'^target$',0)")
        self.db.execute("BEGIN")
        self.prepare()
        self.db.execute("ROLLBACK")
        self.assertEqual(self.db.execute("SELECT ready FROM vocabulary_predicate").fetchone(), (0,))
        self.assertEqual(self.db.execute("SELECT count(*) FROM vocabulary_term").fetchone(), (0,))
        self.assertEqual(self.db.execute("SELECT count(*) FROM vocabulary_pending").fetchone(), (1,))
        self.prepare()
        self.assertEqual(self.db.execute("SELECT term FROM vocabulary_term").fetchone(), ("target",))


class PreparationReportTests(unittest.TestCase):
    def test_equal_relations_in_different_file_batches_are_not_counted_as_avoided_work(self):
        spec = importlib.util.spec_from_file_location("preparation", ROOT / "scripts/profiling/preparation.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        events = [
            {"phase": "anchor-batch", "thread": "a", "startUnixUs": 0, "wallNs": 100_000,
             "detail": {"files": [1]}},
            {"phase": "anchor-batch", "thread": "a", "startUnixUs": 200, "wallNs": 100_000,
             "detail": {"files": [2]}},
            {"phase": "other", "thread": "a", "startUnixUs": 1, "detail": None},
        ]
        for time, cpu in ((10, 5), (20, 6), (210, 8)):
            events.append({"phase": "anchor-materialize", "thread": "a", "startUnixUs": time,
                           "name": "set", "cpuNs": cpu * 1_000_000,
                           "detail": {"relationId": 7, "dependencyClass": "File"}})
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "profile.json").write_text(json.dumps({"exitCode": 0, "timedOut": False}))
            (path / "parity.json").write_text(json.dumps({"nativeSha256": "binary", "workloadSha256": "workload",
                "policyDigests": {}, "falsePositives": [], "falseNegatives": [], "failures": []}))
            (path / "trace.jsonl").write_text("\n".join(map(json.dumps, events)))
            result = module.summarize(path)
        self.assertEqual(result["withinBatchRepeatedCpuMs"], 6)
        self.assertEqual(result["exclusiveCpuMs"]["anchor-materialize"], 19)
        self.assertEqual(result["batchFiles"], [[1], [2]])
        self.assertTrue(result["complete"])


if __name__ == "__main__":
    unittest.main()
