# SPDX-License-Identifier: Apache-2.0
"""Calculate SQLite column demand and omit unused values in authored SQL stages."""
import copy
import re
import sqlite3
from collections import defaultdict

def sql_strings(value):
    if isinstance(value, str):
        yield value
    else:
        for child in value.get("cases", {}).values():
            yield from sql_strings(child)
        if "default" in value:
            yield from sql_strings(value["default"])


def map_sql(value, change):
    if isinstance(value, str):
        return change(value)
    return {key: ({k: map_sql(v, change) for k, v in child.items()} if key == "cases"
                  else map_sql(child, change) if key == "default" else copy.deepcopy(child)) for key, child in value.items()}


def quote(value):
    return '"' + value.replace('"', '""') + '"'


def reads(db, query, schemas, conservative_rowid=True):
    dependencies = set()
    def authorize(action, table, column, database, trigger):
        if action == 20 and table in schemas:  # SQLITE_READ
            dependencies.add(table)
        return 0
    roots = {}
    for database, name in [(0, "main"), (1, "temp")]:
        for table, page in db.execute(f"SELECT name,rootpage FROM {name}.sqlite_schema WHERE type='table'"):
            if table in schemas:
                roots[(database, page)] = table
    db.set_authorizer(authorize)
    try:
        program = list(db.execute("EXPLAIN " + query))
    finally:
        db.set_authorizer(None)
    cursors = defaultdict(set)
    for _, op, p1, p2, p3, *_ in program:
        if op == "OpenRead" and (p3, p2) in roots:
            cursors[p1].add(roots[(p3, p2)])
    for _ in range(len(cursors) + 1):
        for _, op, p1, p2, *_ in program:
            if op == "OpenDup":
                cursors[p1].update(cursors[p2])
    demand = defaultdict(set)
    for _, op, p1, p2, *_ in program:
        if op == "Rowid" and not conservative_rowid:
            continue
        if op in ("Column", "Rowid"):
            for table in cursors[p1]:
                if op == "Rowid" or p2 >= len(schemas[table]):
                    demand[table].update(schemas[table])
                else:
                    demand[table].add(schemas[table][p2])
    for table in dependencies:
        demand[table]  # count(*) still needs a row carrier.
    return demand, dependencies


BASE = ['__path','__start','__end','id','parent','edge_parent','kind','line','text','ordinal','parent_start','parent_end','parent_kind','ancestor_start','ancestor_end','ancestor_kind']


def schema(plan):
    db = sqlite3.connect(':memory:', cached_statements=0)
    for name, arity in [('rx_matches',2),('rx_extract',3),('rx_replace',3),('source_text',3)]:
        db.create_function(name, arity, lambda *args: None)
    tables = {}
    def create(name, columns):
        db.execute(f'CREATE TABLE {quote(name)} ({",".join(map(quote,columns))})')
        tables[name] = columns
    for name, entry in plan['inputs'].items():
        create(name, ['__path','__start','__end',*entry['evidence']['project']])
    spec = plan['syntax']
    cols = [*BASE, *[f'{f}_{s}' for f in spec['fields'] for s in ['id','start','end','kind','source']]]
    create(spec['name'], [c for c in cols if not spec.get('columns') or c in spec['columns']])
    if spec.get('edges'): create(spec['edges'], ['__path','src','dst','ordinal'])
    if spec.get('ownership'): create(spec['ownership'], ['__path','id',*sorted(spec['owners'])])
    for stage in plan['stages']:
        name = stage['name']
        if 'reachability' in stage:
            create(name, ['__path','root','node'])
        else:
            db.execute(f'CREATE TABLE {quote(name)} AS SELECT * FROM ({next(sql_strings(stage["query"]))}) LIMIT 0')
            tables[name] = [r[1] for r in db.execute(f'PRAGMA table_info({quote(name)})')]
            for col in stage.get('deferred', {}):
                if col not in tables[name]:
                    db.execute(f'ALTER TABLE {quote(name)} ADD COLUMN {quote(col)}')
                    tables[name].append(col)
    return db, tables


def narrow_columns(plan):
    # Row carriers and logical width remain intact; internal rowids do not demand all cells.
    assert not any(re.search(r"\b(?:rowid|_rowid_|oid)\b", q, re.I) for s in plan["stages"] for q in sql_strings(s.get("query", "")))
    db, tables = schema(plan)
    required = {n:set() for n in tables}
    def collect(query):
        demand, _ = reads(db, query, tables, conservative_rowid=False)
        for name, cols in demand.items(): required[name].update(cols)
    for query in sql_strings(plan['query']): collect(query)
    result = []
    for stage in reversed(plan['stages']):
        name = stage['name']
        if 'reachability' in stage:
            collect(stage['reachability']['roots'])
            continue
        original = next(sql_strings(stage['query']))
        selected = [c[0] for c in db.execute(f'SELECT * FROM ({original}) LIMIT 0').description]
        wanted = required[name] | {c for index in stage.get('indexes', []) for c in index}
        # Deferred expressions read this stage's stored values, not its parents.
        for expr in stage.get('deferred', {}).values():
            local, _ = reads(db, f'SELECT {expr} FROM {quote(name)}', tables, conservative_rowid=False)
            wanted.update(local.get(name, ()))
        if not wanted: wanted.add(selected[0])
        keep = [c for c in selected if c in wanted]
        dead = [c for c in selected if c not in wanted]
        def change(query):
            # Keep logical schema and multiplicity, but avoid computing unused cells.
            projection = ','.join(quote(c) if c in wanted else f'NULL AS {quote(c)}' for c in selected)
            rewritten = f'SELECT {projection} FROM ({query})'
            if len(rewritten.encode()) > 16384:
                raise ValueError(f'Relational projection exceeds the SQL size limit for {name}')
            actual = ','.join(map(quote, keep)) or '1'
            collect(f'SELECT {actual} FROM ({rewritten})')
            return rewritten
        stage['query'] = map_sql(stage['query'], change)
        if dead: result.append({'stage':name, 'computedBefore':len(selected), 'computedAfter':len(keep), 'nulled':dead})
    db.close()
    return result
