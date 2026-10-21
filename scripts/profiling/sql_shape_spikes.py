#!/usr/bin/env python3
"""Isolated policy controls for the post-preparation SQL shape experiments."""
import argparse
import copy
import json
from pathlib import Path
import re
import sqlite3
from native_sql_plan import reads, quote
from sql_variants import sql_strings, map_sql

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


def prune(plan):
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
                raise ValueError(f'Projection spike exceeds the SQL size limit for {name}')
            actual = ','.join(map(quote, keep)) or '1'
            collect(f'SELECT {actual} FROM ({rewritten})')
            return rewritten
        stage['query'] = map_sql(stage['query'], change)
        if dead: result.append({'stage':name, 'computedBefore':len(selected), 'computedAfter':len(keep), 'nulled':dead})
    db.close()
    return result


def roots(plan, mode):
    changed = 0
    for stage in plan['stages']:
        spec = stage.get('reachability', {})
        text = spec.get('roots', '')
        if ' UNION SELECT ' not in text: continue
        if mode == 'roots-union-all':
            spec['roots'] = text.replace(' UNION SELECT ', ' UNION ALL SELECT ')
        elif mode == 'roots-materialized':
            # Scan the native metadata once; reuse a narrow ordinary SQL CTE.
            columns = sorted(set(['__path','id',*[c for c in plan['syntax']['columns'] if c in re.findall(r'\b[a-z_]\w*\b', re.sub(r"'(?:''|[^'])*'", '', text))]]))
            body = re.sub(r'\bnodes\b', 'seed_nodes', text).replace(' UNION SELECT ',' UNION ALL SELECT ')
            prefix = f'seed_nodes AS MATERIALIZED (SELECT {",".join(columns)} FROM nodes)'
            spec['roots'] = 'WITH RECURSIVE '+prefix+','+body.removeprefix('WITH RECURSIVE ') if body.startswith('WITH RECURSIVE ') else 'WITH '+prefix+' '+body
        else:
            if text.startswith('WITH RECURSIVE roots'):
                predicate = text.split('WHERE ',1)[1].split(' UNION SELECT ',1)[0]
                values = [f'CASE WHEN {predicate} THEN id END', "CASE WHEN kind IN ('assignment','for_statement') THEN left_id END", "CASE WHEN kind IN ('if_statement','elif_clause') THEN condition_id END", "CASE WHEN kind='match_statement' THEN subject_id END", "CASE WHEN parent_kind='conditional_expression' AND ordinal=1 THEN id END"]
                tail = ''
            else:
                first = text.split(' UNION SELECT ',1)[0]
                kinds = first.split('WHERE ',1)[1]
                fields = re.findall(r'SELECT __path,([a-z_]+)_id FROM nodes WHERE \1_start IS NOT NULL',text)
                values = [f'CASE WHEN {kinds} THEN id END', "CASE WHEN kind='for_in_clause' THEN parent END", "CASE WHEN parent_kind='comparison_operator' THEN id END"]
                values += [f'CASE WHEN {f}_start IS NOT NULL THEN {f}_id END' for f in fields]
                tail = ' UNION ALL SELECT __path,id FROM arguments' if 'UNION SELECT __path,id FROM arguments' in text else ''
            def qualify(expr):
                return re.sub(r"'(?:''|[^'])*'|\b[a-z_]\w*\b", lambda m:'n.'+m[0] if m[0] in plan['syntax']['columns'] else m[0],expr)
            spec['roots'] = "SELECT n.__path,j.value AS id FROM nodes n CROSS JOIN json_each(json_array("+','.join(map(qualify,values))+")) j WHERE j.type='integer'"+tail
        changed += 1
    return changed


def variant(document, mode):
    document = copy.deepcopy(document)
    evidence = []
    for clause in document['clauses']:
        plan = clause.get('relational', {})
        if 'syntax' not in plan: continue
        details = []
        if mode in ['method-index','owner-first','joins']:
            for stage in plan['stages']:
                if stage['name'] == 'conditional_methods' and mode != 'owner-first':
                    stage.setdefault('indexes',[]).append(['__path','id'])
                if stage['name'] == 'self_stores' and mode != 'method-index':
                    stage['query'] = stage['query'].replace(' JOIN ownership own ', ' CROSS JOIN ownership own ').replace(' JOIN conditional_methods m ', ' CROSS JOIN conditional_methods m ')
        elif mode in ['return-range','return-bounds']:
            for stage in plan['stages']:
                if stage['name']=='returns': stage.setdefault('indexes',[]).append(['__path','__start','__end'])
                if stage['name']=='integer_returns' and mode=='return-bounds':
                    stage['query']=stage['query'].replace('r.__start>=m.__start AND r.__end<=m.__end','r.__start>=m.__start AND r.__start<=m.__end AND r.__end<=m.__end')
        elif mode == 'projections': details = prune(plan)
        elif mode.startswith('roots-'): details = roots(plan,mode)
        else: raise ValueError(mode)
        evidence.append({'clause':clause['id'],'details':details})
    return document, evidence


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--policy',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    p.add_argument('--variant',action='append',required=True,choices=['method-index','owner-first','joins','return-range','return-bounds','projections','roots-union-all','roots-materialized','roots-unpivot'])
    args=p.parse_args(); document=json.loads(args.policy.read_text()); details=[]
    for mode in args.variant:
        document, changes=variant(document,mode);details.append({'variant':mode,'changes':changes})
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(document,indent=2)+'\n')
    args.output.with_suffix('.experiment.json').write_text(json.dumps(details,indent=2)+'\n')
