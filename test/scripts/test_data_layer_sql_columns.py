"""Semantic regression tests for column demand in generated data-layer policies."""
import copy
from pathlib import Path
import sqlite3
import sys
import unittest
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts/dsl'))
from data_layer_six.sql_columns import narrow_columns, schema

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

    def test_pruning_retains_distinct_multiplicity_aggregate_and_window_inputs(self):
        programs=[
            ([{'name':'a','query':'SELECT DISTINCT id,kind,ordinal FROM nodes'}], 'SELECT id,count(*) FROM a GROUP BY id'),
            ([{'name':'a','query':'SELECT *,row_number() OVER(PARTITION BY kind ORDER BY ordinal) AS position FROM nodes'}], 'SELECT id,position FROM a'),
            ([{'name':'a','query':'SELECT id,sum(ordinal) AS total FROM nodes GROUP BY id'}], 'SELECT id,total FROM a'),
            ([{'name':'a','query':'SELECT * FROM nodes WHERE kind IS NULL'}], 'SELECT count(*) FROM a'),
        ]
        for stages,query in programs:
            original={'inputs':{},'syntax':{'name':'nodes','fields':[],'columns':['__path','__start','__end','id','kind','ordinal'],'owners':{}},'stages':stages,'query':query}
            candidate=copy.deepcopy(original);narrow_columns(candidate)
            rows=[{'id':1,'kind':'a','ordinal':2},{'id':1,'kind':'a','ordinal':3},{'id':2,'kind':None,'ordinal':None}]
            self.assertEqual(self.evaluate(original,rows),self.evaluate(candidate,rows))

    def test_all_query_alternatives_and_deferred_dependencies_retain_their_values(self):
        original={'inputs':{},'syntax':{'name':'nodes','fields':[],
            'columns':['__path','__start','__end','id','kind','ordinal'],'owners':{}},
            'stages':[{'name':'a','query':'SELECT id,kind,ordinal FROM nodes',
                'indexes':[['id']], 'deferred':{'label':"kind||ordinal"}}],
            'query':{'$option':'entry','cases':{'label':'SELECT label FROM a'},'default':'SELECT ordinal FROM a'}}
        candidate=copy.deepcopy(original);narrow_columns(candidate)
        rows=[{'id':1,'kind':'first','ordinal':2},{'id':2,'kind':'second','ordinal':3}]
        for query in ['SELECT label FROM a','SELECT ordinal FROM a']:
            before=copy.deepcopy(original);before['query']=query
            after=copy.deepcopy(candidate);after['query']=query
            self.assertEqual(self.evaluate(before,rows),self.evaluate(after,rows))

if __name__=='__main__':unittest.main()
