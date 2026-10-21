"""Differential ladders for column pruning and finite root-set collection."""
import copy
import json
from pathlib import Path
import sqlite3
import sys
import unittest
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts/profiling'))
from sql_shape_spikes import prune, roots, schema

class ShapeSpikes(unittest.TestCase):
    def evaluate(self, plan, rows):
        db,tables=schema(plan)
        columns=tables['nodes']
        db.executemany('INSERT INTO nodes VALUES('+','.join('?' for _ in columns)+')',[[row.get(c) for c in columns] for row in rows])
        for stage in plan['stages']:
            db.execute('DROP TABLE "'+stage['name']+'"')
            db.execute('CREATE TABLE "'+stage['name']+'" AS '+stage['query'])
        result=sorted(db.execute(plan['query']).fetchall());db.close();return result

    def test_pruning_retains_distinct_multiplicity_aggregate_and_window_inputs(self):
        programs=[
            ([{'name':'a','query':'SELECT DISTINCT id,kind,ordinal FROM nodes'}], 'SELECT id,count(*) FROM a GROUP BY id'),
            ([{'name':'a','query':'SELECT *,row_number() OVER(PARTITION BY kind ORDER BY ordinal) AS position FROM nodes'}], 'SELECT id,position FROM a'),
            ([{'name':'a','query':'SELECT id,sum(ordinal) AS total FROM nodes GROUP BY id'}], 'SELECT id,total FROM a'),
            ([{'name':'a','query':'SELECT * FROM nodes WHERE kind IS NULL'}], 'SELECT count(*) FROM a'),
        ]
        for stages,query in programs:
            original={'inputs':{},'syntax':{'name':'nodes','fields':[],'columns':['__path','__start','__end','id','kind','ordinal'],'owners':{}},'stages':stages,'query':query}
            candidate=copy.deepcopy(original);prune(candidate)
            rows=[{'id':1,'kind':'a','ordinal':2},{'id':1,'kind':'a','ordinal':3},{'id':2,'kind':None,'ordinal':None}]
            self.assertEqual(self.evaluate(original,rows),self.evaluate(candidate,rows))

    def test_every_root_variant_retains_null_zero_parent_and_file_qualification(self):
        policy=json.loads((ROOT/'packs/data-layer/candidates/rules/data-layer-serverplan-quality.json').read_text())
        for clause in policy['clauses']:
            original=clause.get('relational',{})
            if not any('reachability' in s for s in original.get('stages',[])):continue
            db,tables=schema(original)
            rows=[{'id':0,'kind':'module'}, {'id':1,'kind':'comparison_operator'},
                  {'id':2,'kind':'identifier','parent':1,'parent_kind':'comparison_operator','ordinal':1,'name_start':0,'name_id':0},
                  {'id':3,'kind':'conditional_expression'}, {'id':4,'kind':'identifier','parent':3,'parent_kind':'conditional_expression','ordinal':1},
                  {'id':5,'kind':'for_in_clause','parent':6}, {'id':6,'kind':'function_definition','name_source':'require_sql_physical_conditional'},
                  {'id':7,'kind':'assignment','left_id':2,'left_start':0}]
            columns=tables['nodes']
            for path in ['a.py','b.py']:
                db.executemany('INSERT INTO nodes VALUES('+','.join('?' for _ in columns)+')',[[{**r,'__path':path}.get(c) for c in columns] for r in rows])
            stage=next(s for s in original['stages'] if 'reachability' in s)
            def result(plan):
                query=next(s for s in plan['stages'] if 'reachability' in s)['reachability']['roots']
                return sorted(db.execute('SELECT DISTINCT __path,id FROM ('+query+') WHERE __path IS NOT NULL AND id IS NOT NULL').fetchall())
            expected=result(original)
            for mode in ['roots-union-all','roots-materialized','roots-unpivot']:
                plan=copy.deepcopy(original);roots(plan,mode);self.assertEqual(result(plan),expected,(clause['id'],mode))
            db.close()

if __name__=='__main__':unittest.main()
