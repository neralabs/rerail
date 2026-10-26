"""Keep protocol demand ahead of method fan-out in the actual dead-code SQL."""
import json
from pathlib import Path
import sqlite3
import unittest

ROOT = Path(__file__).resolve().parents[2]


class DeadCodeQueries(unittest.TestCase):
    def test_protocol_gate_precedes_unrelated_methods_even_with_skewed_statistics(self):
        policy = json.loads((ROOT / 'packs/data-layer/candidates/rules/data-layer-internal-dead-code.json').read_text())
        relational = next(c['relational'] for c in policy['clauses'] if 'relational' in c)
        edges = next(s['query'] for s in relational['stages'] if s['name'] == 'reach_edges')
        # Execute the actual arm inside its compound materialization boundary.
        arm = 'SELECT m.id AS source,other.id AS target' + edges.rsplit(' UNION SELECT m.id,other.id', 1)[1]
        query = 'WITH edges AS MATERIALIZED (' + ' UNION '.join([
            'SELECT NULL AS source,NULL AS target WHERE 0', arm,
        ]) + ') SELECT * FROM edges'
        for enabled in [False, True]:
            with self.subTest(protocol=enabled), sqlite3.connect(':memory:') as db:
                db.executescript('''
                    CREATE TABLE dead_traits(id,kind,name,class_id,structural_protocol);
                    CREATE INDEX traits_id ON dead_traits(id);
                    CREATE INDEX traits_name ON dead_traits(kind,name);
                    CREATE INDEX traits_class ON dead_traits(class_id,name);
                ''')
                rows = [(f'Class{c}', 'class', f'Class{c}', None, int(enabled and c == 0)) for c in range(700)]
                rows += [(f'Class{c}.method{m}', 'method', f'method{m}', f'Class{c}', 0) for c in range(700) for m in range(3)]
                db.executemany('INSERT INTO dead_traits VALUES(?,?,?,?,?)', rows)
                db.execute('ANALYZE')
                # Sampled statistics can misprice either lookup. Reproduce the late-gate
                # order seen in the full capture without requiring its 385 MiB database.
                db.execute("UPDATE sqlite_stat1 SET stat='2800 13' WHERE idx='traits_id'")
                db.execute('ANALYZE sqlite_schema')
                steps = 0

                def limit():
                    nonlocal steps
                    steps += 1000
                    return steps > 2_000_000

                db.set_progress_handler(limit, 1000)
                actual = set(db.execute(query))
                expected = {(f'Class0.method{m}', f'Class{c}.method{m}') for c in range(1, 700) for m in range(3)} if enabled else set()
                self.assertEqual(actual, expected)
                self.assertLessEqual(steps, 2_000_000)


if __name__ == '__main__':
    unittest.main()
