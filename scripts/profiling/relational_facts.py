"""Compare named SQL producers in two diagnostic SQLite captures, using batched reads."""
import argparse
from collections import Counter
from contextlib import closing
import json
from pathlib import Path
import sqlite3


def identifier(value):
    return '"' + value.replace('"', '""') + '"'


def literal(value):
    return "'" + value.replace("'", "''") + "'"


def compare(left, right):
    paths = [Path(left), Path(right)]
    metadata = [json.loads(path.read_text()) for path in paths]
    schemas = {}
    rows = {}
    with closing(sqlite3.connect(':memory:', uri=True)) as db:
        queries = []
        for index, (path, meta) in enumerate(zip(paths, metadata)):
            if meta['format'] != 'rerail.relational-capture/v1':
                raise ValueError('Expected a relational capture')
            database = path.parent / meta['database']
            db.execute(f'ATTACH DATABASE ? AS d{index}', (database.resolve().as_uri() + '?mode=ro',))
            names = ','.join(literal(name) for name in set(meta['aliases'].values()))
            queries.append(f"SELECT {index},s.name,p.cid,p.name,p.type FROM d{index}.sqlite_schema s CROSS JOIN pragma_table_xinfo(s.name,'d{index}') p WHERE s.type='table' AND s.name IN ({names})")
        for index, table, ordinal, name, kind in db.execute(' UNION ALL '.join(queries)):
            schemas.setdefault((index, table), []).append((ordinal, name, kind))
        queries = []
        for (index, table), columns in schemas.items():
            columns.sort()
            values = ','.join(f"json_array(typeof({identifier(name)}),CASE WHEN typeof({identifier(name)}) IN ('text','blob') THEN hex({identifier(name)}) ELSE quote({identifier(name)}) END)" for _, name, _ in columns)
            queries.append(f'SELECT {index},{literal(table)},json_array({values}) FROM d{index}.{identifier(table)}')
        if queries:
            for index, table, value in db.execute(' UNION ALL '.join(queries)):
                rows.setdefault((index, table), Counter())[value] += 1
    differences = []
    for name in sorted(set(metadata[0]['aliases']) | set(metadata[1]['aliases'])):
        tables = [meta['aliases'].get(name) for meta in metadata]
        columns = [schemas.get((index, table)) for index, table in enumerate(tables)]
        values = [rows.get((index, table), Counter()) for index, table in enumerate(tables)]
        if columns[0] != columns[1] or values[0] != values[1]:
            differences.append({'name': name, 'tables': tables, 'columns': columns,
                                'rowCounts': [sum(v.values()) for v in values],
                                'onlyLeft': list((values[0] - values[1]).items())[:20],
                                'onlyRight': list((values[1] - values[0]).items())[:20]})
    return {'format': 'rerail.relational-comparison/v1', 'captures': list(map(str, paths)),
            'differences': differences, 'sqlReadBatches': 2 if schemas else 1,
            'scope': 'Persisted SQLite rows only; native modules are recorded as bindings, not copied row vectors.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('left', type=Path)
    parser.add_argument('right', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    report = json.dumps(compare(args.left, args.right), indent=2) + '\n'
    if args.output:
        args.output.write_text(report)
    else:
        print(report, end='')
