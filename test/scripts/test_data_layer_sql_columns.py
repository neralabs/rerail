"""Semantic regression tests for column demand in generated data-layer policies."""
import copy
import json
from pathlib import Path
import sqlite3
import sys
import unittest
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts/dsl'))
from data_layer_six.sql_columns import compact_columns, narrow_columns, schema

class SqlColumns(unittest.TestCase):
    def evaluate(self, plan, rows):
        db,tables=schema(plan)
        columns=tables['nodes']
        db.executemany('INSERT INTO nodes VALUES('+','.join('?' for _ in columns)+')',[[row.get(c) for c in columns] for row in rows])
        for stage in plan['stages']:
            db.execute('DROP TABLE "'+stage['name']+'"')
            name=stage['name']
            stored=name+'_stored' if stage.get('deferred') else name
            db.execute('CREATE TABLE "'+stored+'" AS '+stage['query'])
            if stage.get('deferred'):
                columns=','.join(expr+' AS "'+name+'"' for name,expr in stage['deferred'].items())
                db.execute('CREATE VIEW "'+name+'" AS SELECT *,'+columns+' FROM "'+stored+'"')
        result=sorted(db.execute(plan['query']).fetchall());db.close();return result

    def test_repository_conditional_functions_store_only_demanded_columns(self):
        policy=json.loads((ROOT/'packs/data-layer/candidates/rules/data-layer-serverplan-quality.json').read_text())
        plan=next(c['relational'] for c in policy['clauses'] if c['id']=='quality-conditional-1')
        db=sqlite3.connect(':memory:');self.addCleanup(db.close)
        for name,arity in [('rx_matches',2),('rx_extract',3),('rx_replace',3)]:db.create_function(name,arity,lambda *args:None)
        for name,entry in plan['inputs'].items():
            cols=['__path','__start','__end',*entry['evidence']['project']]
            db.execute('CREATE TABLE "'+name+'" ('+','.join('"'+c+'"' for c in cols)+')')
        for stage in plan['stages']:
            if 'reachability' in stage:db.execute('CREATE TABLE "'+stage['name']+'" (__path,root,node)')
            else:
                name=stage['name'];stored=name+'_stored' if stage.get('deferred') else name
                db.execute('CREATE TABLE "'+stored+'" AS '+stage['query']+' LIMIT 0')
                if stage.get('deferred'):
                    values=','.join(expr+' AS "'+col+'"' for col,expr in stage['deferred'].items())
                    db.execute('CREATE VIEW "'+name+'" AS SELECT *,'+values+' FROM "'+stored+'"')
        columns=lambda name:[r[1] for r in db.execute('PRAGMA table_info("'+name+'")')]
        self.assertLessEqual(len(columns('functions_stored')), 24)
        self.assertLessEqual(len(columns('classes_stored')), 24)
        self.assertLessEqual(len(columns('conditional_methods_stored')), 26)
        self.assertIn('parameters_kind',columns('functions'))
        self.assertIn('qualified_name',columns('conditional_classes'))

    def test_pruning_retains_distinct_multiplicity_aggregate_and_window_inputs(self):
        programs=[
            ([{'name':'a','query':'SELECT DISTINCT id,kind,ordinal FROM nodes'}], 'SELECT id,count(*) FROM a GROUP BY id'),
            ([{'name':'a','query':'SELECT *,row_number() OVER(PARTITION BY kind ORDER BY ordinal) AS position FROM nodes'}], 'SELECT id,position FROM a'),
            ([{'name':'a','query':'SELECT id,sum(ordinal) AS total FROM nodes GROUP BY id'}], 'SELECT id,total FROM a'),
            ([{'name':'a','query':'SELECT * FROM nodes WHERE kind IS NULL'}], 'SELECT count(*) FROM a'),
        ]
        for stages,query in programs:
            original={'inputs':{},'syntax':{'name':'nodes','fields':[],'columns':['__path','__start','__end','id','kind','ordinal'],'owners':{}},'stages':stages,'query':query}
            rows=[{'id':1,'kind':'a','ordinal':2},{'id':1,'kind':'a','ordinal':3},{'id':2,'kind':None,'ordinal':None}]
            for project in [narrow_columns,compact_columns]:
                for native in [True,False]:
                    before=copy.deepcopy(original)
                    if not native:
                        del before['syntax']
                        before['inputs']={'nodes':{'evidence':{'project':{'id':'id','kind':'kind','ordinal':'ordinal'}}}}
                    candidate=copy.deepcopy(before);project(candidate)
                    self.assertEqual(self.evaluate(before,rows),self.evaluate(candidate,rows))

    def test_all_query_alternatives_and_deferred_dependencies_retain_their_values(self):
        original={'inputs':{},'syntax':{'name':'nodes','fields':[],
            'columns':['__path','__start','__end','id','kind','ordinal'],'owners':{}},
            'stages':[{'name':'a','query':'SELECT id,kind,ordinal FROM nodes',
                'indexes':[['id']], 'deferred':{'label':"kind||ordinal"}}],
            'query':{'$option':'entry','cases':{'label':'SELECT label FROM a'},'default':'SELECT ordinal FROM a'}}
        rows=[{'id':1,'kind':'first','ordinal':2},{'id':2,'kind':'second','ordinal':3}]
        for project in [narrow_columns,compact_columns]:
            candidate=copy.deepcopy(original);project(candidate)
            for query in ['SELECT label FROM a','SELECT ordinal FROM a']:
                before=copy.deepcopy(original);before['query']=query
                after=copy.deepcopy(candidate);after['query']=query
                self.assertEqual(self.evaluate(before,rows),self.evaluate(after,rows))

if __name__=='__main__':unittest.main()
