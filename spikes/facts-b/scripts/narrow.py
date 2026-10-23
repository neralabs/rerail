"""Narrowings of the order-dependent repository constructs (docs/facts/NARROWINGS.md).

narrow.py OUT_DIR [--diag] [NAME=VALUE ...]
Writes edited copies of the rule files to OUT_DIR (NAME=VALUE sets an option's default). With
--diag, each edited stage also gets a clause `<clause>--diag-<stage>` holding the ORIGINAL stages
up to it, its `<stage>__narrowed` copy (over the original upstream stages) and, for row_number
stages, `__rank`/`__nrank` copies (row_number -> rank, original and narrowed), whose query reports
one finding (one per file for a file-partitioned clause) with code
`DIAG:<clause>:<stage>:diff=<rows in exactly one side>:orig=<n>:narrowed=<n>[:ties=<n>:narrowed_ties=<n>]`
(ties: rows tied at the top of a partition beyond the first, between distinct output rows).
Run the copies through native's SQLite path with `conformance/data-layer/suite.ts --policies OUT_DIR`.
"""
import copy, json, sys, os

RULES = os.path.join(os.path.dirname(os.path.abspath(__file__)), '../../../packs/data-layer/candidates/rules')

# (rule file, clause, stage, [(old, new, count)])
DECL = ('row_number() OVER (PARTITION BY id ORDER BY __start DESC) AS selected FROM inventory',
        'row_number() OVER (PARTITION BY id ORDER BY __start DESC,__path) AS selected FROM inventory', 1)
IMPORTS = [
    ('tokens(path,ordinal,top,base,item,rest) AS (', 'tokens(path,ordinal,top,base,item,rest,position) AS (', 1),
    ("SELECT __path,__start,top,base,'',rx_replace(items,'#[^\\n]*','') || ',' FROM raw_imports\n UNION ALL SELECT path,ordinal,top,base,trim(substr(rest,1,instr(rest,',')-1),' ()' || char(10) || char(13)),substr(rest,instr(rest,',')+1) FROM tokens WHERE rest<>''",
     "SELECT __path,__start,top,base,'',rx_replace(items,'#[^\\n]*','') || ',',0 FROM raw_imports\n UNION SELECT path,ordinal,top,base,trim(substr(rest,1,instr(rest,',')-1),' ()' || char(10) || char(13)),substr(rest,instr(rest,',')+1),position+1 FROM tokens WHERE rest<>''", 1),
    ('relative(path,ordinal,top,base,original,alias,module,prefix,remaining) AS (\n SELECT path,ordinal,top,base,original,alias,module,',
     'relative(path,ordinal,top,base,original,alias,module,prefix,remaining,position) AS (\n SELECT path,ordinal,top,base,original,alias,module,', 1),
    ("coalesce(length(base)-length(ltrim(base,'.'))-1,-1) FROM parts\n UNION ALL SELECT path,ordinal,top,base,original,alias,module,rx_replace(prefix,'\\.?[^.]+$',''),remaining-1 FROM relative WHERE remaining>0)",
     "coalesce(length(base)-length(ltrim(base,'.'))-1,-1),position FROM parts\n UNION SELECT path,ordinal,top,base,original,alias,module,rx_replace(prefix,'\\.?[^.]+$',''),remaining-1,position FROM relative WHERE remaining>0)", 1),
    ('PARTITION BY module,name,base IS NULL ORDER BY ordinal DESC)', 'PARTITION BY module,name,base IS NULL ORDER BY ordinal DESC,position DESC,path)', 1),
]
BASE_EDGES = [
    ("i.name=trim(rx_replace(rx_replace(r.name,'(?s)\\[.*$',''),'^.*\\.','')) LIMIT 1)", "i.name=trim(rx_replace(rx_replace(r.name,'(?s)\\[.*$',''),'^.*\\.','')) ORDER BY c.id LIMIT 1)", 1),
    ("c.name=trim(rx_replace(rx_replace(r.name,'(?s)\\[.*$',''),'^.*\\.','')) LIMIT 1)", "c.name=trim(rx_replace(rx_replace(r.name,'(?s)\\[.*$',''),'^.*\\.','')) ORDER BY c.id LIMIT 1)", 1),
]
HIERARCHY = [
    ("SELECT id,id,0,'',',' || id || ',' FROM declarations WHERE kind='class' UNION ALL", "SELECT id,id,0,'',',' || id || ',' FROM declarations WHERE kind='class' UNION", 1),
]
PUBLIC_METHODS = [('PARTITION BY h.owner,d.name ORDER BY h.depth,h.route) AS selected', 'PARTITION BY h.owner,d.name ORDER BY h.depth,h.route,d.id) AS selected', 1)]
CLASS_NAMES = [
    ("WHERE i.kind='symbol' AND i.module=r.module AND i.name=r.name LIMIT 1)", "WHERE i.kind='symbol' AND i.module=r.module AND i.name=r.name ORDER BY c.id LIMIT 1)", 1),
    ("c.module=r.module AND c.name=r.name LIMIT 1)", "c.module=r.module AND c.name=r.name ORDER BY c.id LIMIT 1)", 1),
]
TRAITS = [('FROM parameters p WHERE p.symbol=d.id ORDER BY p.__start)', 'FROM parameters p WHERE p.symbol=d.id ORDER BY p.__start,p.__path)', 1)]
SLOTS = [('PARTITION BY scope,owner,base_name,slot ORDER BY ordinal DESC) AS selected', 'PARTITION BY scope,owner,base_name,slot ORDER BY ordinal DESC,symbol) AS selected', 1)]
FAMILY_GROUPS = [('ORDER BY x.first_ordinal LIMIT 1) AS exemplar', 'ORDER BY x.first_ordinal,x.slot LIMIT 1) AS exemplar', 1)]
TRIPLE = [('PARTITION BY f.family ORDER BY d.__path,d.__start) AS selected', 'PARTITION BY f.family ORDER BY d.__path,d.__start,s.symbol) AS selected', 1)]
BLOCK_RESULTS = [("SELECT DISTINCT __path,0,'{}' FROM contract_blocks UNION ALL SELECT", "SELECT DISTINCT __path,0,'{}' FROM contract_blocks UNION SELECT", 1)]
COND_FINDINGS = [("a.__start<n.__start ORDER BY a.__start DESC LIMIT 1)", "a.__start<n.__start ORDER BY a.__start DESC,a.origin LIMIT 1)", 1)]
DIRECT_SINKS = [("n.__start) AS chosen FROM direct_events d JOIN nodes n", "n.__start,d.node) AS chosen FROM direct_events d JOIN nodes n", 1)]
TRANSITIVE_SINKS = [
    ("FROM direct_sinks s JOIN reachable r ON r.__path=s.__path AND r.fn=s.fn UNION ALL SELECT", "FROM direct_sinks s JOIN reachable r ON r.__path=s.__path AND r.fn=s.fn UNION SELECT", 1),
    ('ORDER BY length(s.route),n.__start) AS chosen', 'ORDER BY length(s.route),n.__start,s.node,s.route) AS chosen', 1),
]
PROJECT_SINKS = [
    ("FROM project_seeds UNION ALL SELECT", "FROM project_seeds UNION SELECT", 1),
    ('ORDER BY length(s.route),s.priority,n.__start) AS chosen', 'ORDER BY length(s.route),s.priority,n.__start,s.node,s.route) AS chosen', 1),
]
SHARED = {'declarations': [DECL], 'imports': IMPORTS, 'base_edges': BASE_EDGES, 'hierarchy': HIERARCHY, 'public_methods': PUBLIC_METHODS}
EDITS = {
    ('data-layer-internal-dead-code.json', 'unreachable-internal-symbols'): {**SHARED, 'class_names': CLASS_NAMES},
    ('data-layer-public-api-triples.json', 'exported-families'): {**SHARED, 'traits': TRAITS, 'slots': SLOTS, 'family_groups': FAMILY_GROUPS, 'triple_violations': TRIPLE},
    ('data-layer-serverplan-quality.json', 'quality-conditional-1'): {'block_results': BLOCK_RESULTS, 'conditional_findings_0': COND_FINDINGS},
    ('data-layer-serverplan-state-tripwires.json', 'state-flow-1'): {'direct_sinks': DIRECT_SINKS, 'transitive_sinks': TRANSITIVE_SINKS},
    ('data-layer-serverplan-state-tripwires.json', 'state-flow-project-1'): {'direct_sinks': DIRECT_SINKS, 'project_sinks': PROJECT_SINKS},
}
# families (`row_number() … ORDER BY owner`): no text change; one row per (scope, owner,
# base_name) comes out of family_groups' GROUP BY, so the order is already total.


def apply(text, edits, where):
    for old, new, n in edits:
        c = text.count(old)
        if c != n:
            raise SystemExit(f'{where}: expected {n} occurrence(s) of {old[:80]!r}, found {c}')
        text = text.replace(old, new)
    return text


def edit_query(q, edits, where):
    """A stage query is text or an `$option` object whose cases are texts."""
    if isinstance(q, str):
        return apply(q, edits, where)
    raise SystemExit(f'{where}: $option stage query not handled')


def main():
    out = sys.argv[1]
    diag = '--diag' in sys.argv
    options = dict(a.split('=', 1) for a in sys.argv[2:] if '=' in a)
    os.makedirs(out, exist_ok=True)
    by_file = {}
    for (f, cid), st in EDITS.items():
        by_file.setdefault(f, {})[cid] = st
    for f, clauses in by_file.items():
        d = json.load(open(os.path.join(RULES, f)))
        for name, value in options.items():
            opts = d['clauses'][0]['define']['options']
            if name in opts:
                opts[name]['default'] = value
        extra = []
        for c in d['clauses']:
            if c['id'] not in clauses:
                continue
            rel = c['relational']
            original = copy.deepcopy(c)
            names = [s['name'] for s in rel['stages']]
            for s in rel['stages']:
                if s['name'] in clauses[c['id']]:
                    s['query'] = edit_query(s['query'], clauses[c['id']][s['name']], f'{c["id"]}/{s["name"]}')
            missing = set(clauses[c['id']]) - set(names)
            if missing:
                raise SystemExit(f'{c["id"]}: stages not found {missing}')
            if diag:
                extra.extend(diag_clause(original, name, edits) for name, edits in clauses[c['id']].items())
        d['clauses'].extend(extra)
        json.dump(d, open(os.path.join(out, f), 'w'), indent=1)
        print('wrote', f, 'clauses', list(clauses), 'diag' if diag else '')


# (clause, stage) -> (probe query over the stage's inputs, ties query)
PROBES = {
    ('exported-families', 'traits'): (
        "SELECT d.id,coalesce((SELECT json_group_array(name) FROM (SELECT name FROM parameters p WHERE p.symbol=d.id ORDER BY p.__start)),'[]') AS params FROM declarations d",
        "SELECT count(*) FROM (SELECT symbol,__start FROM parameters GROUP BY symbol,__start HAVING count(DISTINCT name)>1)"),
}

def diag_clause(c, name, edits):
    """One clause per edited stage: the original stages up to it, its narrowed copy (over the
    original upstream stages) and, for row_number stages, its rank() copies. The finding anchors
    to the clause's first input row (native requires an input witness span)."""
    c = copy.deepcopy(c)
    cid = c['id']
    c['id'] = f'{cid}--diag-{name}'
    rel = c['relational']
    names = [s['name'] for s in rel['stages']]
    stages = rel['stages'][:names.index(name) + 1]
    s = stages[-1]
    # partitionBy kept: a file-partitioned clause reports one diag row per file (summed after).
    ties = ''
    if (cid, name) in PROBES:
        # A stage too heavy to run twice in one clause: compare only the column the edit changes.
        orig_q, ties_q = PROBES[(cid, name)]
        stages = stages[:-1] + [{'name': name, 'query': orig_q}]
        added = [{'name': name + '__narrowed', 'query': apply(orig_q, edits, f'{cid}/{name} probe')}]
        ties = f"||':ties='||({ties_q})"
    else:
        narrowed = {k: v for k, v in s.items() if k != 'indexes'}
        narrowed['name'] = name + '__narrowed'
        narrowed['query'] = edit_query(s['query'], edits, f'{cid}/{name}')
        added = [narrowed]
    o, nn = name, name + '__narrowed'
    if 'row_number()' in s['query']:
        # Ties under the original order (SQLite broke them by input order) and under the
        # narrowed one (each would be a runtime refusal in the dataflow), between distinct rows.
        for suffix, q in (('__rank', s['query']), ('__nrank', added[0]['query'])):
            r = {k: v for k, v in s.items() if k != 'indexes'}
            r['name'] = name + suffix
            r['query'] = q.replace('row_number()', 'rank()')
            added.append(r)
        ties = (f"||':ties='||((SELECT count(*) FROM (SELECT DISTINCT * FROM {name}__rank))-(SELECT count(*) FROM (SELECT DISTINCT * FROM {o})))"
                f"||':narrowed_ties='||((SELECT count(*) FROM (SELECT DISTINCT * FROM {name}__nrank))-(SELECT count(*) FROM (SELECT DISTINCT * FROM {nn})))")
    rel['stages'] = stages + added
    inp = next(iter(rel['inputs']))
    rel['query'] = (
        f"SELECT a.__path AS path,a.__start AS start_byte,a.__end AS end_byte,'DIAG:{cid}:{name}:diff='||"
        f"(SELECT count(*) FROM (SELECT * FROM (SELECT * FROM {o} EXCEPT SELECT * FROM {nn}) UNION ALL SELECT * FROM (SELECT * FROM {nn} EXCEPT SELECT * FROM {o})))"
        f"||':orig='||(SELECT count(*) FROM {o})||':narrowed='||(SELECT count(*) FROM {nn}){ties} AS diag"
        f" FROM (SELECT __path,__start,__end FROM {inp} ORDER BY __path,__start LIMIT 1) a")
    rel['key'] = ['diag']
    rel['code'] = 'diag'
    rel.pop('locations', None)
    return c


if __name__ == '__main__':
    main()
