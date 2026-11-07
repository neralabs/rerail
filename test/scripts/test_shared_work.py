"""Sharing diagnostics must reject false equivalence and explain conflicting names."""
from copy import deepcopy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/profiling"))
from shared_work.capture import discover
from shared_work.contracts import check


def contract():
    return {"name": "syntax.source_spans.v1", "completion": "keys",
            "inputs": {"tape": "syntax-tape"}, "options": {},
            "columns": [{"name": name, "type": "integer", "nullable": False}
                        for name in ("fid", "pre", "start", "length")],
            "key": ["fid", "pre"], "multiplicity": "set", "ordering": [],
            "implementation": {"backend": "sql", "template": "source-spans-v1"},
            "origin": "decode.rs:10"}


def binding(identifier="a"):
    return {"id": identifier, "contract": "syntax.source_spans.v1",
            "inputs": {"tape": {"external": {"name": "python-tape", "generation": "file-1-generation-2"}}},
            "options": {}, "consumers": [f"rule-{identifier}"], "demand": [[1, 2]],
            "origin": f"rule-{identifier}.json:20"}


def manifest():
    return {"format": "rerail.shared-work-plan/v1", "snapshot": "evaluation-1",
            "contracts": [contract()], "bindings": [binding(), binding("b")]}


def capture(queries, stages=(), parameters=()):
    return {"format": "rerail.candidate-replay/v2", "queries": queries,
            "stages": list(stages), "parameters": list(parameters)}


class ContractTests(unittest.TestCase):
    def test_different_demand_and_consumer_names_share_one_binding(self):
        plan = manifest()
        plan["bindings"][1]["demand"] = [[2, 3], [2, 4]]
        report = check([("rules.json", plan)])
        self.assertTrue(report["valid"])
        self.assertEqual(len(report["sharedBindings"]), 1)
        self.assertEqual(len(report["sharedBindings"][0]["bindings"]), 2)

    def test_contract_clash_has_both_origins_and_exact_field(self):
        first, second = manifest(), manifest()
        second["contracts"][0]["columns"][2]["nullable"] = True
        report = check([("first.json", first), ("second.json", second)])
        self.assertFalse(report["valid"])
        clash = report["clashes"][0]
        self.assertEqual(clash["firstOrigin"]["file"], "first.json")
        self.assertEqual(clash["secondOrigin"]["file"], "second.json")
        self.assertEqual(clash["differences"][0]["field"], "columns[2].nullable")
        self.assertEqual(report["sharedBindings"], [])

    def test_all_contract_semantics_participate_in_clash_detection(self):
        for field, value in (("implementation", {"backend": "native", "function": "other"}),
                             ("key", ["fid"]), ("completion", "domain"),
                             ("multiplicity", "bag"),
                             ("ordering", [{"column": "fid", "direction": "desc", "nulls": "last"}]),
                             ("options", {"field": "integer"}), ("inputs", {"other": "syntax-tape"})):
            with self.subTest(field=field):
                first, second = manifest(), manifest()
                second["contracts"][0][field] = value
                report = check([("first", first), ("second", second)])
                self.assertTrue(report["clashes"])
                self.assertTrue(any(d["field"].startswith(field)
                                    for d in report["clashes"][0]["differences"]))

    def test_different_options_are_valid_separate_bindings(self):
        plan = manifest()
        plan["contracts"][0]["options"] = {"field": "integer"}
        plan["bindings"][0]["options"] = {"field": {"kind": "integer", "value": 3}}
        plan["bindings"][1]["options"] = {"field": {"kind": "integer", "value": 4}}
        report = check([("rules", plan)])
        self.assertTrue(report["valid"])
        self.assertFalse(report["sharedBindings"])
        self.assertEqual(report["separateBindings"][0]["differences"][0]["field"], "options.field.value")

    def test_boolean_is_not_an_integer_parameter(self):
        plan = manifest()
        plan["contracts"][0]["options"] = {"field": "integer"}
        for item in plan["bindings"]:
            item["options"] = {"field": {"kind": "integer", "value": True}}
        self.assertFalse(check([("rules", plan)])["valid"])

    def test_different_input_generation_never_shares(self):
        plan = manifest()
        plan["bindings"][1]["inputs"]["tape"]["external"]["generation"] = "generation-3"
        report = check([("rules", plan)])
        self.assertTrue(report["valid"])
        self.assertFalse(report["sharedBindings"])
        self.assertEqual(report["separateBindings"][0]["differences"][0]["field"],
                         "inputs.tape.external.generation")

    def test_input_bindings_resolve_by_identity_instead_of_local_name(self):
        plan = manifest()
        consumer = deepcopy(contract())
        consumer.update(name="syntax.span_lengths.v1", inputs={"spans": "span-relation"},
                        implementation={"backend": "sql", "template": "lengths-v1"})
        plan["contracts"].append(consumer)
        for name, parent in (("c", "a"), ("d", "b")):
            item = binding(name)
            item.update(contract="syntax.span_lengths.v1", inputs={"spans": parent})
            plan["bindings"].append(item)
        report = check([("rules", plan)])
        self.assertTrue(report["valid"])
        self.assertEqual(len(report["sharedBindings"]), 2)

    def test_clashing_local_binding_id_is_an_error(self):
        plan = manifest()
        plan["bindings"][1]["id"] = "a"
        plan["bindings"][1]["inputs"]["tape"]["external"]["name"] = "different-tape"
        report = check([("rules", plan)])
        self.assertFalse(report["valid"])
        self.assertEqual(report["clashes"][0]["kind"], "binding-name-clash")

    def test_domain_membership_is_semantic_and_required(self):
        plan = manifest()
        plan["contracts"][0]["completion"] = "domain"
        self.assertFalse(check([("rules", plan)])["valid"])
        for i, item in enumerate(plan["bindings"]):
            item["domain"] = {"files": [1, i + 2], "generations": ["g1", "g2"]}
        report = check([("rules", plan)])
        self.assertTrue(report["valid"])
        self.assertFalse(report["sharedBindings"])

    def test_cycle_and_missing_input_have_actionable_errors(self):
        for target, expected in (("a", "cyclic binding"), ("unknown", "missing input binding")):
            with self.subTest(target=target):
                plan = manifest()
                plan["bindings"][0]["inputs"]["tape"] = target
                report = check([("rules", plan)])
                self.assertFalse(report["valid"])
                self.assertTrue(any(expected in e["message"] for e in report["errors"]))

    def test_hash_collision_cannot_merge_distinct_bindings(self):
        plan = manifest()
        plan["bindings"][1]["inputs"]["tape"]["external"]["name"] = "other-tape"
        with patch("shared_work.contracts.digest", return_value="collision"):
            report = check([("rules", plan)])
        self.assertTrue(report["valid"])
        self.assertFalse(report["sharedBindings"])
        self.assertEqual(len(report["separateBindings"]), 1)

    def test_unknown_semantic_fields_fail_instead_of_disappearing(self):
        plan = manifest()
        plan["bindings"][0]["scope"] = "other"
        report = check([("rules", plan)])
        self.assertFalse(report["valid"])
        self.assertIn("unknown fields", report["errors"][0]["message"])

    def test_snapshots_remain_distinct_across_manifests(self):
        first, second = manifest(), manifest()
        first["bindings"] = [binding()]
        second["bindings"] = [binding()]
        second["snapshot"] = "evaluation-2"
        report = check([("first", first), ("second", second)])
        self.assertTrue(report["valid"])
        self.assertFalse(report["sharedBindings"])
        self.assertEqual(report["separateBindings"][0]["differences"][0]["field"], "snapshot")

    def test_registration_order_does_not_change_identity_digest(self):
        plan = manifest()
        other = binding("other")
        other["inputs"]["tape"]["external"]["name"] = "other-tape"
        plan["bindings"].append(other)
        first = check([("rules", plan)])
        plan["bindings"].reverse()
        second = check([("rules", plan)])
        self.assertEqual(first["sharedBindings"][0]["identitySha256"],
                         second["sharedBindings"][0]["identitySha256"])

    def test_equivalent_names_are_suggestions_only(self):
        plan = manifest()
        other = deepcopy(contract())
        other["name"] = "syntax.also_spans.v1"
        plan["contracts"].append(other)
        plan["bindings"][1]["contract"] = other["name"]
        report = check([("rules", plan)])
        self.assertTrue(report["valid"])
        self.assertFalse(report["sharedBindings"])
        self.assertEqual(len(report["equivalentContractCandidates"]), 1)


class CaptureTests(unittest.TestCase):
    def report(self, *queries, stages=(), parameters=()):
        return discover(capture(queries, stages, parameters), "test.json")

    def test_renamed_resolved_graphs_match(self):
        report = self.report(
            "WITH needed(x) AS MATERIALIZED (VALUES(1)), decoded(x) AS MATERIALIZED (SELECT x FROM needed) SELECT * FROM decoded",
            "WITH renamed(x) AS MATERIALIZED (VALUES(1)), other(x) AS MATERIALIZED (SELECT x FROM renamed) SELECT * FROM other")
        self.assertTrue(report["complete"])
        self.assertEqual(report["definitionsInMultipleHosts"], 2)

    def test_same_local_name_with_different_dependencies_is_not_shared(self):
        report = self.report(
            "WITH needed(x) AS MATERIALIZED (VALUES(1)), decoded(x) AS MATERIALIZED (SELECT x FROM needed) SELECT * FROM decoded",
            "WITH needed(x) AS MATERIALIZED (VALUES(2)), decoded(x) AS MATERIALIZED (SELECT x FROM needed) SELECT * FROM decoded")
        self.assertEqual(report["definitionsInMultipleHosts"], 0)
        self.assertEqual(report["localNamesWithDifferentDefinitions"], 2)

    def test_typed_parameters_do_not_alias(self):
        report = self.report("WITH a(x) AS MATERIALIZED (SELECT ?1) SELECT * FROM a",
                             "WITH b(x) AS MATERIALIZED (SELECT ?2) SELECT * FROM b",
                             parameters=[{"kind": "integer", "value": 1}, {"kind": "text", "value": "1"}])
        self.assertEqual(report["definitionsInMultipleHosts"], 0)

    def test_literals_and_comments_do_not_become_dependencies(self):
        report = self.report(
            "WITH needed(x) AS MATERIALIZED (VALUES(1)), a(x) AS MATERIALIZED (SELECT 'needed') SELECT * FROM a",
            "WITH needed(x) AS MATERIALIZED (VALUES(2)), b(x) AS MATERIALIZED (SELECT 'needed') SELECT * FROM b")
        self.assertEqual(report["definitionsInMultipleHosts"], 1)
        quoted = self.report("WITH a(x) AS MATERIALIZED (SELECT '(' /* ) */) SELECT * FROM a",
                             "WITH b(x) AS MATERIALIZED (SELECT '(' /* ) */) SELECT * FROM b")
        self.assertTrue(quoted["complete"])
        self.assertEqual(quoted["definitionsInMultipleHosts"], 1)

    def test_mutable_write_is_a_sharing_boundary(self):
        stages = [{"name": "first", "temporary": False,
                   "sql": "WITH a(x) AS MATERIALIZED (SELECT x FROM facts) INSERT INTO output SELECT * FROM a"},
                  {"name": "change", "temporary": False, "sql": "UPDATE facts SET x=2"}]
        report = self.report("WITH b(x) AS MATERIALIZED (SELECT x FROM facts) SELECT * FROM b", stages=stages)
        self.assertEqual(report["definitionsInMultipleHosts"], 0)

    def test_quoted_mutable_table_keeps_its_write_boundary(self):
        stages = [{"name": "first", "temporary": False,
                   "sql": 'WITH a(x) AS MATERIALIZED (SELECT x FROM "facts") INSERT INTO output SELECT * FROM a'},
                  {"name": "change", "temporary": False, "sql": 'UPDATE "facts" SET x=2'}]
        report = self.report('WITH b(x) AS MATERIALIZED (SELECT x FROM "facts") SELECT * FROM b', stages=stages)
        self.assertEqual(report["definitionsInMultipleHosts"], 0)

    def test_physical_output_projection_participates_in_identity(self):
        stages = [{"name": name, "temporary": True, "columns": "x",
                   "sql": f"CREATE TEMP TABLE {name} AS WITH a(x) AS MATERIALIZED (VALUES(1)) SELECT {expression} AS x FROM a"}
                  for name, expression in (("p", "x"), ("q", "CAST(x AS TEXT)"))]
        report = self.report("WITH b(x) AS MATERIALIZED (SELECT x FROM p) SELECT * FROM b",
                             "WITH c(x) AS MATERIALIZED (SELECT x FROM q) SELECT * FROM c", stages=stages)
        self.assertEqual(report["definitionsInMultipleHosts"], 1)  # Only VALUES(1).

    def test_unsupported_nested_scope_is_explicitly_incomplete(self):
        report = self.report("WITH a(x) AS MATERIALIZED (WITH b(x) AS MATERIALIZED (VALUES(1)) SELECT * FROM b) SELECT * FROM a")
        self.assertFalse(report["complete"])
        self.assertIn("nested WITH", report["skippedHosts"][0]["reason"])

    def test_hash_collision_does_not_merge_different_ctes(self):
        with patch("shared_work.capture.digest", return_value="collision"):
            report = self.report("WITH a(x) AS MATERIALIZED (VALUES(1)) SELECT * FROM a",
                                 "WITH b(x) AS MATERIALIZED (VALUES(2)) SELECT * FROM b")
        self.assertEqual(report["definitionsInMultipleHosts"], 0)


class CommandTests(unittest.TestCase):
    def test_clash_fails_command_and_writes_reviewable_report(self):
        with tempfile.TemporaryDirectory() as directory:
            source, output = Path(directory) / "plan.json", Path(directory) / "report.json"
            plan = manifest()
            conflicting = deepcopy(contract())
            conflicting["multiplicity"] = "bag"
            plan["contracts"].append(conflicting)
            source.write_text(json.dumps(plan))
            result = subprocess.run([sys.executable, str(ROOT / "scripts/profiling/shared_work.py"),
                                     "check", str(source), "--output", str(output)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertFalse(json.loads(output.read_text())["valid"])


if __name__ == "__main__":
    unittest.main()
