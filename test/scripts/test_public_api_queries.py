"""Bound the actual public-API SQL paths observed in the full repository capture."""
import json
from pathlib import Path
import re
import sqlite3
import unittest

ROOT = Path(__file__).resolve().parents[2]
POLICY = ROOT / 'packs/data-layer/candidates/rules/data-layer-public-api-triples.json'


class PublicApiQueries(unittest.TestCase):
    def setUp(self):
        policy = json.loads(POLICY.read_text())
        relational = next(c['relational'] for c in policy['clauses'] if 'relational' in c)
        self.stages = {s['name']: s for s in relational['stages']}
        self.db = sqlite3.connect(':memory:')
        self.db.create_function('rx_matches', 2, lambda text, rx: int(bool(re.search(rx, text or ''))))
        self.db.create_function('rx_replace', 3, lambda text, rx, by: re.sub(rx, by, text or ''))
        self.addCleanup(self.db.close)

    def stage(self, name):
        s = self.stages[name]
        self.db.execute(f'CREATE TABLE "{name}" AS {s["query"]}')
        for i, fields in enumerate(s.get('indexes', [])):
            self.db.execute(f'CREATE INDEX "{name}_{i}" ON "{name}" ({",".join(fields)})')

    def test_owner_comparison_uses_both_columns_of_the_declared_index(self):
        self.db.execute('CREATE TABLE raw_decorators(__path TEXT,__start INT,__end INT,owner,name)')
        self.db.execute('CREATE TABLE declaration(__path TEXT,__start INT)')
        self.db.executemany('INSERT INTO raw_decorators VALUES(?,?,?,?,?)', [('a.py', i, i+1, i, 'property') for i in range(1000)])
        self.stage('decorators')
        plan = self.db.execute('EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM decorators x WHERE x.__path=d.__path AND x.owner=d.__start) FROM declaration d').fetchall()
        self.assertTrue(any('__path=? AND owner=?' in row[3] for row in plan), plan)

    def test_exported_class_demand_does_not_scan_all_bases_per_method(self):
        self.db.executescript('''
          CREATE TABLE raw_bases(__path TEXT,__start INT,__end INT,owner,name);
          CREATE TABLE declarations(__path TEXT,__start INT,id TEXT,module TEXT,name TEXT,class_name TEXT,kind TEXT);
          CREATE INDEX decl_id ON declarations(id);
          CREATE INDEX decl_module_class ON declarations(module,class_name);
          CREATE TABLE hierarchy(owner TEXT,base TEXT,depth INT,route TEXT);
          CREATE INDEX hierarchy_base ON hierarchy(base);
          CREATE TABLE traits(__path TEXT,__start INT,id TEXT,name TEXT,kind TEXT,legacy_async INT,execution_surface INT,private_api INT);
          CREATE INDEX traits_id ON traits(id);
          CREATE TABLE exported_roots(symbol TEXT,export_module TEXT,export_name TEXT);
          CREATE INDEX exported_symbol ON exported_roots(symbol);
          CREATE TABLE decorators(__path TEXT,owner INT,name TEXT);
          CREATE INDEX decorator_owner ON decorators(__path,owner);
        ''')
        for i in range(1000):
            owner = f'Class{i}'
            self.db.execute('INSERT INTO raw_bases VALUES(?,?,?,?,?)', ('classes.py', i, i+1, i, 'Base'))
            self.db.execute('INSERT INTO declarations VALUES(?,?,?,?,?,?,?)', ('classes.py', i, owner, 'classes', owner, None, 'class'))
            self.db.execute('INSERT INTO hierarchy VALUES(?,?,0,\'\')', (owner, owner))
            self.db.execute('INSERT INTO traits VALUES(?,?,?,?,?,?,?,?)', ('classes.py', i, owner, owner, 'class', 0, 0, 0))
            for name in ['execute', 'execute_async']:
                symbol = f'{owner}.{name}'
                self.db.execute('INSERT INTO declarations VALUES(?,?,?,?,?,?,?)', ('classes.py', i, symbol, 'classes', name, owner, 'method'))
                self.db.execute('INSERT INTO traits VALUES(?,?,?,?,?,?,?,?)', ('classes.py', i, symbol, name, 'method', 0, 1, 0))
        self.db.execute("INSERT INTO exported_roots VALUES('Class0','data_layer.api','Class0')")
        if 'bases' in self.stages:
            self.stage('bases')
        self.stage('public_methods')
        self.db.execute('ANALYZE')
        steps = 0
        def limit():
            nonlocal steps
            steps += 1000
            return steps > 2_000_000
        self.db.set_progress_handler(limit, 1000)
        rows = self.db.execute(self.stages['surfaces']['query']).fetchall()
        self.assertEqual(len(rows), 2)
        self.assertLessEqual(steps, 2_000_000)


if __name__ == '__main__':
    unittest.main()
