"""Execute all SQL stages in one shared WITH scope around native boundaries."""
import argparse
import copy
import json
from pathlib import Path
import re

from policies import map_sql, strings


def pipeline(plan):
    stages = plan.get('stages', [])
    sql_stages = [stage for stage in stages if 'query' in stage]
    native = [stage for stage in stages if 'query' not in stage]
    names = {stage['name'] for stage in sql_stages}
    # A native input that depends on SQL is a real boundary, not an inlineable edge.
    for stage in native:
        for query in stage.get('reachability', {}).values():
            if isinstance(query, str) and names.intersection(re.findall(r'\b\w+\b', query)):
                raise ValueError('A native producer requires a preceding SQL scope')
    macros = [stage['query'] for stage in sql_stages if not isinstance(stage['query'], str)]
    if len(macros) > 1 or any('template' not in macro for macro in macros):
        raise ValueError('This spike requires at most one independent template option per pipeline')
    prefix = ','.join(stage['name'] + ' AS MATERIALIZED (' +
                      (stage['query'] if isinstance(stage['query'], str) else stage['query']['template']) + ')'
                      for stage in sql_stages)
    def emit(query):
        # The one template remains an option expression, expanded by the normal compiler.
        # Other option cases stay outside it. Literal placeholders cannot be conflated.
        if macros and ('{}' in query or any('{}' in stage['query']
                                            for stage in sql_stages if isinstance(stage['query'], str))):
            raise ValueError('A literal placeholder conflicts with the template option')
        body = 'WITH RECURSIVE ' + prefix + ' SELECT * FROM (' + query + ')'
        if not macros:
            return body
        result = copy.deepcopy(macros[0])
        result['template'] = body
        return result
    plan['query'] = map_sql(plan['query'], emit)
    plan['stages'] = native
    lengths = [len(value) for value in strings(plan['query'])]
    return {'inlined': [stage['name'] for stage in sql_stages],
            'retained': [stage['name'] for stage in native],
            'largestQueryBytes': max(lengths)}


def generate(source, output):
    output.mkdir(parents=True)
    receipt = {}
    for path in sorted(source.glob('*.json')):
        document = json.loads(path.read_text())
        for clause in document['clauses']:
            plan = clause.get('relational', {})
            if list(plan.get('inputs', {})) == ['events']:
                plan['requireNonemptyInput'] = 'events'
            if document['id'] == 'data-layer-serverplan-state-tripwires' and plan.get('syntax'):
                receipt[clause['id']] = pipeline(plan)
        (output / path.name).write_text(json.dumps(document, indent=2) + '\n')
    (output.parent / 'shared-pipelines.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=Path('packs/data-layer/candidates/rules'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(generate(args.source, args.output), indent=2))
