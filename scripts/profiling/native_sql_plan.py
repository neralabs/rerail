"""Experimental SQLite-driven column demand and last-consumer planning.

Empty schema tables preserve the logical interface. EXPLAIN supplies actual column
reads; the authorizer supplies schema dependencies needed even by dead expressions.
Physical tables retain demanded columns; views retain other columns as NULL.
This is a lab compiler, not a policy equivalence or SQL side-effect guarantee.
"""
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "dsl"))
from data_layer_six.sql_columns import quote, reads


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
