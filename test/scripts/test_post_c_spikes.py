"""Semantic checks for the isolated post-C SQL prototypes."""
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, "scripts/profiling")
from post_c_spikes import candidates, policies, affinity


class PolicyTests(unittest.TestCase):
    def test_private_stage_keeps_one_materialization_inside_its_only_consumer(self):
        plan = {"stages": [{"name": "private", "query": "SELECT value FROM input", "indexes": [["value"]]}],
                "query": "SELECT * FROM private a JOIN private b USING(value)"}
        removed = policies.private_stages(plan)
        self.assertEqual([row['name'] for row in removed], ['private'])
        self.assertEqual(plan['stages'], [])
        self.assertEqual(plan['query'].count('private AS MATERIALIZED'), 1)
        db = sqlite3.connect(':memory:')
        db.executescript('CREATE TABLE input(value); INSERT INTO input VALUES(1),(1),(2)')
        self.assertEqual(db.execute(plan['query']).fetchall(), [(1,), (1,), (1,), (1,), (2,)])
        db.close()

    def test_template_consumers_count_and_keep_multiply_consumed_producers(self):
        plan = {'stages': [{'name': 'shared', 'query': 'SELECT value FROM input'},
                           {'name': 'a', 'query': 'SELECT * FROM shared'},
                           {'name': 'b', 'query': {'$option': 'config', 'template': "SELECT * FROM shared WHERE value='{}'"}}],
                'query': 'SELECT * FROM a UNION ALL SELECT * FROM b'}
        policies.private_stages(plan)
        self.assertIn('shared', [stage['name'] for stage in plan['stages']])

    def test_native_and_recursive_boundaries_remain_and_template_literals_do_not_change(self):
        plan = {'stages': [{'name': 'roots', 'query': 'SELECT __path,id FROM nodes'},
                           {'name': 'walk', 'reachability': {'syntax': 'nodes', 'roots': 'SELECT * FROM roots'}},
                           {'name': 'recursive', 'query': 'WITH RECURSIVE w(x) AS (VALUES(1)) SELECT * FROM w'}],
                'query': 'SELECT * FROM recursive'}
        self.assertEqual(policies.private_stages(plan), [])
        plan = {'stages': [{'name': 'literal', 'query': "SELECT '{}' AS value"}],
                'query': {'$option': 'config', 'template': "SELECT * FROM literal WHERE value='{}'"}}
        self.assertEqual(policies.private_stages(plan), [])


class CandidateTests(unittest.TestCase):
    def test_generated_cte_boundaries_respect_quotes_and_nested_functions(self):
        sql = "WITH RECURSIVE a(x) AS MATERIALIZED (SELECT 'x,)''y'),b(y) AS NOT MATERIALIZED (SELECT printf('%s)',x) FROM a) SELECT * FROM b"
        definitions, body = candidates.split_query(sql)
        self.assertEqual(len(definitions), 2)
        self.assertEqual(body, 'SELECT * FROM b')
        self.assertEqual(candidates.fields(definitions[1])[0], 'b')

    def test_set_folding_preserves_duplicate_bags_and_empty_difference(self):
        for operation, left, right in [('UNION', 'rA', 'r0'), ('EXCEPT', 'rA', 'r0'), ('INTERSECT', 'rA', 'r0'), ('EXCEPT', 'r0', 'rA')]:
            with self.subTest(operation=operation, left=left), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                db = sqlite3.connect(root / 'input.sqlite')
                db.executescript('CREATE TABLE input(fid INTEGER,pre INTEGER); INSERT INTO input VALUES(1,7),(1,7)')
                db.commit()
                graph = ['rA(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM input)',
                         'r0(fid,pre) AS MATERIALIZED (SELECT 0 AS fid,0 AS pre WHERE 0)',
                         f'rB(fid,pre) AS MATERIALIZED (SELECT fid,pre FROM {left} {operation} SELECT fid,pre FROM {right})']
                truth = db.execute('WITH RECURSIVE ' + ','.join(graph) + ' SELECT 0,fid,pre FROM rB UNION ALL SELECT 1,fid,pre FROM rA').fetchall()
                aliases = 'rA(fid,pre) AS NOT MATERIALIZED (SELECT fid,pre FROM candidate_keys WHERE producer=0),rB(fid,pre) AS NOT MATERIALIZED (SELECT fid,pre FROM candidate_keys WHERE producer=1)'
                data = {'database': 'input.sqlite', 'graph': graph, 'setup': ['CREATE TEMP TABLE candidate_keys(producer INTEGER,fid INTEGER,pre INTEGER)'],
                        'parameters': [], 'stages': [
                            {'name': 'rA', 'slot': 0, 'members': [0], 'temporary': True, 'sql': 'WITH RECURSIVE ' + graph[0] + ' INSERT INTO candidate_keys SELECT 0,fid,pre FROM rA'},
                            {'name': 'rB', 'slot': 1, 'members': [1], 'temporary': True, 'sql': 'WITH RECURSIVE ' + ','.join(graph) + ' INSERT INTO candidate_keys SELECT 1,fid,pre FROM rB'}],
                        'queries': ['WITH RECURSIVE ' + aliases + ' SELECT 0,fid,pre FROM rB UNION ALL SELECT 1,fid,pre FROM rA'],
                        'schedule': [{'Prepare': 0}, {'Prepare': 1}, {'Query': 0}, {'Release': 1}, {'Release': 0}]}
                capture = root / 'capture.json'
                capture.write_text(json.dumps(data))
                output = root / 'rewritten' / 'capture.json'
                candidates.rewrite(capture, output, False)
                rewritten = json.loads(output.read_text())
                db.execute(rewritten['setup'][0])
                for action in rewritten['schedule']:
                    if 'Prepare' in action:
                        db.execute(rewritten['stages'][action['Prepare']]['sql'])
                    if 'Query' in action:
                        actual = db.execute(rewritten['queries'][action['Query']]).fetchall()
                self.assertEqual(sorted(actual), sorted(truth))
                db.close()


class SchemaFixtureTests(unittest.TestCase):
    def test_frozen_syntax_declarations_keep_production_order_and_types(self):
        declarations = affinity.inputs({'inputs': {}, 'syntax': {
            'name': 'nodes', 'fields': ['name'], 'columns': ['name_source', '__end', '__path', 'id', '__start'],
            'edges': 'edges', 'ownership': 'ownership', 'owners': {'fn': {}, 'cls': {}}}})
        self.assertEqual(declarations['nodes'], 'CREATE TABLE "nodes"("__path" TEXT,"__start" INTEGER,"__end" INTEGER,"id","name_source")')
        self.assertIn('"__path" TEXT,"id","cls","fn"', declarations['ownership'])


if __name__ == '__main__':
    unittest.main()
