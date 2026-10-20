"""Experimental SQLite-driven column demand and last-consumer planning.

Empty schema tables preserve the logical interface. EXPLAIN supplies actual column
reads; the authorizer supplies schema dependencies needed even by dead expressions.
Physical tables retain demanded columns; views retain other columns as NULL.
This is a lab compiler, not a policy equivalence or SQL side-effect guarantee.
"""
from collections import defaultdict


def quote(value):
    return '"' + value.replace('"', '""') + '"'


def reads(db, query, schemas):
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
        if op in ("Column", "Rowid"):
            for table in cursors[p1]:
                if op == "Rowid" or p2 >= len(schemas[table]):
                    demand[table].update(schemas[table])
                else:
                    demand[table].add(schemas[table][p2])
    for table in dependencies:
        demand[table]  # count(*) still needs a row carrier.
    return demand, dependencies


def plan(db, inputs, stages, final_query, additional=None):
    schemas = dict(inputs)
    for name, columns in inputs.items():
        db.execute(f"CREATE TABLE {quote(name)} ({','.join(quote(c) for c in columns)})")
    for stage in stages:
        name = stage["name"]
        db.execute(f"CREATE TEMP TABLE {quote(name)} AS SELECT * FROM ({stage['query']}) LIMIT 0")
        schemas[name] = [row[1] for row in db.execute(f"PRAGMA table_info({quote(name)})")]
    required, dependencies = reads(db, final_query, schemas)
    last_use = {name: len(stages) for name in dependencies}
    projections = {}
    for index in range(len(stages) - 1, -1, -1):
        stage = stages[index]
        name = stage["name"]
        wanted = required[name]
        for fields in stage.get("indexes", []):
            wanted.update(fields)
        if not wanted:
            wanted.add(schemas[name][0])
        columns = [c for c in schemas[name] if c in wanted]
        projections[name] = columns
        query = "SELECT " + ",".join(quote(c) for c in columns) + " FROM (" + stage["query"] + ")"
        queries = [query, *(additional or {}).get(name, [])]
        for query in queries:
            demand, dependencies = reads(db, query, schemas)
            for table, fields in demand.items():
                required[table].update(fields)
            for table in dependencies:
                last_use[table] = max(last_use.get(table, -1), index)
    for name, columns in inputs.items():
        projections[name] = [c for c in columns if c in required[name]] or [columns[0]]
    return {"schemas": schemas, "columns": projections, "lastUse": last_use,
            "logicalColumns": sum(map(len, schemas.values())), "physicalColumns": sum(map(len, projections.values()))}


def materialize(db, name, query, schemas, columns, indexes=()):
    physical = "__physical_" + name
    projection = ",".join(quote(c) for c in columns)
    db.execute(f"CREATE TEMP TABLE {quote(physical)} AS SELECT {projection} FROM ({query})")
    expose(db, name, physical, schemas, columns)
    for i, fields in enumerate(indexes):
        db.execute(f"CREATE INDEX {quote(physical+'_'+str(i))} ON {quote(physical)} ({','.join(quote(f) for f in fields)})")
    if indexes:
        db.execute("ANALYZE " + quote(physical))


def expose(db, name, physical, schema, columns):
    wanted = set(columns)
    select = ",".join(quote(c) if c in wanted else "NULL AS " + quote(c) for c in schema)
    db.execute(f"CREATE TEMP VIEW {quote(name)} AS SELECT {select} FROM {quote(physical)}")


def release(db, compiled, index):
    dropped = []
    for name, last in compiled["lastUse"].items():
        if last != index:
            continue
        db.execute("DROP VIEW " + quote(name))
        db.execute("DROP TABLE " + quote("__physical_" + name))
        dropped.append(name)
    return dropped
