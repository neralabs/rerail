"""Freeze actual cold compiler programs for the SQLite metadata batching host."""
import argparse
import json
from pathlib import Path
import re


def quote(name):
    return '"' + name.replace('"', '""') + '"'


def declaration(name, columns):
    return 'CREATE TABLE ' + quote(name) + '(' + ','.join(quote(column) + (' ' + kind if kind else '') for column, kind in columns) + ')'


def inputs(plan):
    result = {}
    for name, relation in plan['inputs'].items():
        fields = sorted(set(relation['evidence'].get('project', {})) - {'path', 'start_byte', 'end_byte'})
        result[name] = declaration(name, [('__path', 'TEXT'), ('__start', 'INTEGER'), ('__end', 'INTEGER'), *[(field, '') for field in fields]])
    spec = plan['syntax']
    base = ['__path', '__start', '__end', 'id', 'subtree_end', 'parent', 'edge_parent', 'kind', 'line', 'text', 'ordinal', 'parent_start', 'parent_end', 'parent_kind', 'ancestor_start', 'ancestor_end', 'ancestor_kind']
    for field in spec['fields']:
        base.extend(field + '_' + suffix for suffix in ['id', 'start', 'end', 'kind', 'source'])
    if spec.get('columns'):
        base = [name for name in base if name in spec['columns']]
    types = lambda names: [(name, 'TEXT' if name == '__path' else 'INTEGER' if name in ['__start', '__end'] else '') for name in names]
    result[spec['name']] = declaration(spec['name'], types(base))
    if spec.get('edges'):
        result[spec['edges']] = declaration(spec['edges'], types(['__path', 'src', 'dst', 'ordinal']))
    if spec.get('ownership'):
        result[spec['ownership']] = declaration(spec['ownership'], types(['__path', 'id', *sorted(spec.get('owners', {}))]))
    return result


def generate(raw, policy, output):
    policy = json.loads(policy.read_text())
    programs = []
    cold = re.compile(r'CREATE TEMP TABLE "(\w+)" AS SELECT \* FROM \((.*)\) LIMIT 0$', re.S)
    for path in sorted(raw.glob('profile-*.json')):
        catalog = json.loads(path.read_text())
        queries = {query['id']: query['sql'] for query in catalog['queries']}
        events = [json.loads(line) for line in path.with_suffix('.jsonl').read_text().splitlines()]
        events = [event for event in events if event.get('kind') == 'sql' and event.get('origin') == 'engine' and (event.get('context') or {}).get('scope') == 'compile']
        if not events:
            continue
        index = int(events[0]['context']['path'].split('/')[2])
        clause = policy['clauses'][index]
        known = {}
        for event in events:
            match = cold.fullmatch(queries[event['query']])
            if not match:
                raise ValueError(queries[event['query']][:100])
            known[match[1]] = match[2]
        plan = clause['relational']
        stages = []
        for stage in plan['stages']:
            if stage.get('reachability'):
                query = 'SELECT CAST(NULL AS TEXT) AS __path,NULL AS root,NULL AS node WHERE 0'
            else:
                query = stage['query']
                if not isinstance(query, str):
                    query = known.get(stage['name'])
                    if query is None:
                        query = next(p['templateQueries'][stage['name']] for p in programs if stage['name'] in p['templateQueries'])
            stages.append({'name': stage['name'], 'query': query})
        programs.append({'name': path.stem + ':' + clause['id'], 'inputs': inputs(plan), 'stages': stages, 'templateQueries': known, 'capturedColdStatements': len(events)})
    if not programs:
        raise ValueError('No cold inference programs in this capture')
    output.write_text(json.dumps(programs, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--raw', type=Path, required=True)
    parser.add_argument('--policy', type=Path, default=Path('packs/data-layer/candidates/rules/data-layer-serverplan-state-tripwires.json'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    generate(args.raw, args.policy, args.output)
