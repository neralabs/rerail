#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate seventh-batch declarative candidates from the pinned source contracts."""
from __future__ import annotations
import argparse
import ast
import json
from pathlib import Path
import re

PIN = '2b1504adc4291daaf080b2e932962b134da1a69a'
ROOT = Path(__file__).resolve().parents[2]


def constant(source, name):
    tree = ast.parse(source.read_text())
    for node in tree.body:
        if isinstance(node, (ast.Assign, ast.AnnAssign)):
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            if any(isinstance(t, ast.Name) and t.id == name for t in targets):
                value = node.value
                if isinstance(value, ast.Call) and isinstance(value.func, ast.Name) and value.func.id == 'frozenset':
                    value = value.args[0]
                return ast.literal_eval(value)
    raise ValueError(f'Missing literal {source}:{name}')


def any_of(*rules): return {'any': list(rules)}
def all_of(*rules): return {'all': list(rules)}
def field(name, rule): return {'has': {'field': name, **rule}}
def descendant(rule): return {'has': {**rule, 'stopBy': 'end'}}
def identifier(pattern): return {'kind': 'identifier', 'regex': pattern}
def attribute(pattern): return {'kind': 'attribute', **field('attribute', identifier(pattern))}
def string(values): return all_of({'kind': 'string'}, {'value': {'regex': '^(?:' + '|'.join(map(re.escape, sorted(values))) + ')$'}})
def name_set(values): return '^(?:' + '|'.join(map(re.escape, sorted(values))) + ')$'
def call(name, args=None):
    rule = all_of({'kind': 'call'}, field('function', any_of(identifier(name), attribute(name))))
    if args is not None: rule['all'].append(field('arguments', args))
    return rule


def policy(name, scope):
    return {'format': 'rerail.policy/v1', 'id': 'data-layer-' + name.replace('_', '-'), 'revision': 1,
            'mode': 'block', 'description': f'Candidate port of nera-data-layer {name} at {PIN}.',
            'scope': {'include': scope, 'exclude': ['**/tests/**', '**/test/**', '**/conform/**', '**/.git/**', '**/.venv/**', '**/__pycache__/**', '**/.pytest_cache/**', '**/node_modules/**', '**/tmp/**', '**/browser_build/**']}, 'clauses': []}


def add(p, name, rule, code):
    p['clauses'].append({'id': name, 'forbid': {'ast': {'language': 'python', 'syntax': 'valid', 'rule': rule}},
                         'violation': {'code': code, 'anchor': 'witness.span', 'message': code}})


def generate(source):
    out = []
    upstream = source / 'scripts'
    p = policy('publication_registry_writes', ['impl/**/*.py', 'data_layer/**/*.py', '_internal/**/*.py', 'l[0-9][0-9][0-9][0-9]_*/**/*.py'])
    registry = attribute('^cte_refs$')
    indexed = all_of({'kind': 'subscript'}, field('value', registry))
    owner = {'filePath': {'regex': r'(?:^|/)l2070_batching/compilation\.py$'}}
    assignment = all_of(any_of({'kind': 'assignment'}, {'kind': 'augmented_assignment'}), field('left', indexed))
    deletion = all_of({'kind': 'delete_statement'}, any_of({'has': indexed}, {'has': all_of({'kind': 'expression_list'}, {'has': indexed})}))
    mutators = constant(upstream / 'lint_publication_registry_writes.py', 'MUTATING_METHODS')
    method = all_of({'kind': 'call'}, field('function', all_of(attribute(name_set(mutators)), field('object', registry))))
    add(p, 'direct-write', all_of(any_of(assignment, deletion, method), {'not': owner}), 'REGISTRY_DIRECT_WRITE')
    out.append(p)

    p = policy('unknown_contract_defaults', ['impl/**/*.py', 'data_layer/**/*.py', '_internal/**/*.py', 'l[0-9][0-9][0-9][0-9]_*/**/*.py'])
    fields = constant(upstream / 'lint_unknown_contract_defaults.py', 'THREE_STATE_FIELDS')
    builders = constant(upstream / 'lint_unknown_contract_defaults.py', 'EMPTY_BUILDERS')
    contract = attribute(name_set(fields))
    empty = any_of(*[all_of({'kind': k}, {'regex': r'^' + opening + r'(?:\s|#[^\n]*(?:\n|$))*' + closing + r'$'})
        for k, opening, closing in [('tuple', r'\(', r'\)'), ('list', r'\[', r'\]'), ('dictionary', r'\{', r'\}')]], string(['']),
        all_of(call(name_set(builders)), field('function', {'kind': 'identifier'}),
               field('arguments', {'regex': r'^\((?:\s|#[^\n]*(?:\n|$))*\)$'})))
    default = all_of({'kind': 'boolean_operator'}, field('operator', {'regex': '^or$'}), field('left', contract), field('right', empty))
    add(p, 'or-empty', default, 'UNKNOWN_DEFAULTED')
    add(p, 'not', all_of({'kind': 'not_operator'}, field('argument', contract)), 'UNKNOWN_AS_FALSY')
    positional = all_of({'not': {'kind': 'keyword_argument'}}, {'not': {'kind': 'comment'}}, {'not': {'regex': r'^[(),]$'}})
    bool_call = all_of({'kind': 'call'}, field('function', identifier('^bool$')),
                      field('arguments', all_of({'has': contract}, {'count': {'has': positional, 'min': 1, 'max': 1}})))
    add(p, 'bool', bool_call, 'UNKNOWN_AS_FALSY')
    out.append(p)

    p = policy('declared_fact_interrogation', ['impl/**/*.py', 'data_layer/**/*.py', '_internal/**/*.py', 'l[0-9][0-9][0-9][0-9]_*/**/*.py'])
    names = constant(upstream / 'lint_declared_fact_interrogation.py', 'CONTRACT_ATTRIBUTES')
    builtin = all_of({'kind': 'call'}, field('function', identifier('^(?:getattr|hasattr)$')),
                     field('arguments', {'has': {**string(names), 'nthChild': 2}}))
    add(p, 'contract-attribute', builtin, 'CONTRACT_ATTR_PROBE')
    # The Python oracle compares its first operand with every comparator.
    # It deliberately does not advance `left` through a chained comparison.
    typed = all_of({'kind': 'attribute'}, {'pattern': 'type($$$SUBJECT).$A'},
                   {'not': field('object', field('arguments', {'regex': r'^\(\s*self(?:\s*,)?\s*\)$'}))})
    other = all_of({'kind': 'attribute'}, {'pattern': '$OBJECT.$B'})
    pairs = any_of(
        all_of({'has': all_of(typed, {'nthChild': 1})},
               {'has': all_of(other, {'not': {'nthChild': 1}}, {'compare': {'left': 'A', 'op': 'eq', 'right': 'B'}})}),
        all_of({'has': all_of(other, {'nthChild': 1})},
               {'has': all_of(typed, {'not': {'nthChild': 1}}, {'compare': {'left': 'A', 'op': 'eq', 'right': 'B'}})}))
    probe = all_of({'kind': 'comparison_operator'},
                   {'not': {'has': {'regex': r'^(?:==|!=|<|>|<=|>=|<>|in|not in)$'}}},
                   pairs)
    add(p, 'override-identity', probe, 'OVERRIDE_IDENTITY_PROBE')
    out.append(p)

    p = policy('history_serverplan_projection', ['impl/**/*.py', 'l7200_versioned_transactions/**/*.py'])
    production = all_of({'filePath': {'regex': r'^(?:impl|l7200_versioned_transactions)/'}}, {'filePath': {'notRegex': r'(?:^|/)tests/|(?:^|/)test_[^/]*$'}})
    history = {'filePath': {'regex': r'^impl/common/l2070_server_plan/steps/(?:.*/)?[^/]*history[^/]*\.py$'}}
    add(p, 'from-imports', all_of(production, {'kind': 'import_from_statement'},
        field('module_name', {'regex': '^impl\\.common\\.l2070_server_plan\\.steps\\.history_runtime_projection$'})), 'HSP-001')
    imported = all_of({'kind': 'dotted_name', 'regex': r'history_runtime_projection$', 'pattern': '$IMPORTED'})
    imports = all_of(production, {'kind': 'import_statement'},
        any_of({'has': imported}, {'has': all_of({'kind': 'aliased_import'}, field('name', imported))}))
    add(p, 'imports', imports, 'HSP-001')
    p['clauses'][-1]['violation']['each'] = 'IMPORTED'
    returns = all_of({'kind': 'return_statement'}, any_of({'has': {'kind': 'string'}}, {'has': any_of({'kind': 'set'}, {'kind': 'tuple'}, {'kind': 'list'})}, {'has': all_of(call('^frozenset$'), field('function', {'kind': 'identifier'}))}))
    add(p, 'history-runtime', all_of(production, history, returns, descendant(string(['history'])), descendant(string(['runtime']))), 'HSP-002')
    add(p, 'sentinel', all_of(production, history, returns, descendant(string(['__history_runtime__']))), 'HSP-003')
    add(p, 'post-op-refs', all_of(production, history, call('^CompiledStep$'), field('arguments', {'has': {'kind': 'keyword_argument', **field('name', identifier('^post_op_with_refs$'))}})), 'HSP-004')
    out.append(p)

    p = policy('grammar_mutation_invalidation', ['impl/common/l2100_sql_grammar/**/*.py'])
    names = constant(upstream / 'lint_grammar_mutation_invalidation.py', 'MEMOIZABLE_GRAMMAR_CLASSES')
    constructors = constant(upstream / 'lint_grammar_mutation_invalidation.py', 'CONSTRUCTION_METHODS')
    mutators = constant(upstream / 'lint_grammar_mutation_invalidation.py', 'COLLECTION_MUTATORS')
    private = all_of(attribute('^_'), field('object', identifier('^self$')))
    target = {'matches': 'private-target'}
    p['clauses'].append({'id': 'targets', 'define': {'language': 'python', 'utils': {
        'private-target': any_of(private, all_of({'kind': 'subscript'}, field('value', {'matches': 'private-target'})))}}})
    mutation = any_of(all_of({'kind': 'named_expression'}, field('name', target)), all_of(any_of({'kind': 'assignment'}, {'kind': 'augmented_assignment'}), field('left', target)),
                      all_of({'kind': 'delete_statement'}, {'has': target}),
                      all_of(call(name_set(mutators)), field('function', field('object', target))))
    render = all_of({'kind': 'decorated_definition'}, {'has': all_of({'kind': 'function_definition'}, field('name', identifier('^render$')))},
                    {'has': all_of({'kind': 'decorator'}, {'regex': 'memoize_in_read_only_scope$'})})
    owner = all_of({'kind': 'class_definition'}, any_of(field('name', identifier(name_set(names))), field('body', {'has': render})))
    owner = all_of(owner, {'pattern': '$OWNER'})
    # Only direct class members are methods. Nested functions still contribute
    # mutations and self-calls to their containing method, as ast.walk does.
    membership = any_of(
        {'inside': all_of({'kind': 'block'}, {'inside': owner})},
        {'inside': all_of({'kind': 'decorated_definition'}, {'inside': all_of({'kind': 'block'}, {'inside': owner})})})
    method = all_of({'kind': 'function_definition'}, field('name', {'kind': 'identifier', 'pattern': '$METHOD'}), membership)
    subject = all_of(method, {'not': field('name', identifier(name_set(constructors)))},
                     descendant(mutation), {'not': descendant(call('^forget_memoized_render_fragments$'))})
    self_call = all_of({'kind': 'call'}, field('function', all_of(attribute('.*'),
        field('object', identifier('^self$')), field('attribute', {'pattern': '$CALLEE'}))))
    graph = all_of(method, descendant(self_call))
    def relation(alias, query, project, **extra):
        return {'evidence': {'as': alias, 'query': {'ast': {'language': 'python', 'syntax': 'valid', 'rule': query}},
                             'project': project, **extra}}
    p['clauses'].append({'id': 'mutation', 'require_each': {
        'subject': relation('mutator', subject, {'path': 'file.path', 'owner': 'capture.OWNER.startByte', 'method': 'capture.METHOD'}),
        'evidence': relation('edge', graph,
            {'path': 'file.path', 'owner': 'capture.OWNER.startByte', 'caller': 'capture.METHOD', 'callee': 'capture.CALLEE',
             'private_callee': {'source': 'capture.CALLEE', 'regex': '^(_.*)$', 'group': 1},
             'public_caller': {'source': 'capture.METHOD', 'regex': '^([^_].*)$', 'group': 1}},
            each='CALLEE', closure=[
                {'name': 'construction', 'seeds': sorted(constructors), 'partition_by': ['path', 'owner'],
                 'edges': {'from': 'caller', 'to': 'private_callee'}, 'contains': {'from_construction': 'callee'}},
                {'name': 'runtime', 'seed_field': 'public_caller', 'partition_by': ['path', 'owner'],
                 'edges': {'from': 'caller', 'to': 'callee'}, 'contains': {'from_runtime': 'callee'}},
            ], where={'all': [{'eq': ['edge.path', 'mutator.path']}, {'eq': ['edge.owner', 'mutator.owner']},
                             {'eq': ['edge.callee', 'mutator.method']}, {'eq': ['edge.from_construction', True]},
                             {'eq': ['edge.from_runtime', False]}]})},
        'violation': {'code': 'MUTATION_WITHOUT_INVALIDATION', 'anchor': 'witness.span', 'message': 'Mutation without memo invalidation'}})
    out.append(p)

    p = policy('revision_ancestry_invariants', ['impl/common/l2100_sql_grammar/domain/versioning/**/*.py', 'impl/common/l2070_server_plan/steps/**/*.py'])
    p['clauses'].append({'id': 'boundary', 'define': {'options': {'oracleAnalyzer': {'type': 'boolean', 'default': False}}}})
    def text_finding(id, pattern, code, message, *, capture=None):
        query = {'source': 'file.content', 'pattern': pattern,
                 'content': {'include': [{'$option': 'oracleAnalyzer', 'cases': {'true': '(?s).*', 'false': 'revision|ancestry|stmt_ancestry_chain'}}]},
                 'paths': {'include': ['**/*.py'], 'exclude': ['**/tests/**', '**/test_*.py']}}
        anchor = {'$option': 'oracleAnalyzer', 'cases': {'true': 'file.start', 'false': 'capture.' + capture}} if capture else 'witness.span'
        p['clauses'].append({'id': id, 'forbid': {'regex': query}, 'violation': {'code': code, 'anchor': anchor, 'message': message}})
    names = constant(upstream / 'lint_revision_ancestry_invariants.py', 'FORBIDDEN_CLASS_NAMES')
    for name in sorted(names):
        text_finding('descriptor-' + name.lstrip('_'), r'\A[\s\S]*?(?P<DESCRIPTOR>(?m:^class\s+' + re.escape(name) + r'\b))',
                     'forbidden_descriptor', 'forbidden ancestry descriptor ' + name, capture='DESCRIPTOR')
    text_finding('removed-surface', r'\A[\s\S]*?(?P<SURFACE>stmt_ancestry_chain)', 'removed_surface', 'Removed ancestry surface', capture='SURFACE')
    columns = '(?:' + '|'.join(constant(upstream / 'lint_revision_ancestry_invariants.py', 'REVISION_COLUMNS')) + ')'
    for i, pattern in enumerate([columns + r"""[\"'`]?[\s\S]{0,80}::\s*text""", r'CAST\([\s\S]{0,160}?' + columns + r"""[\"'`]?[\s\S]{0,80}?\s+AS\s+(?:TEXT|CHAR(?:\s*\(\s*\d+\s*\))?)\s*\)"""]):
        text_finding(f'stored-text-cast-{i}', '(?i)' + pattern, 'stored_revision_text_cast', 'Stored revision column cast to text')
    out.append(p)
    from data_layer_batch_seven_identity import candidate as merged_identity
    out.append(merged_identity(source, policy, constant))
    from data_layer_batch_seven_dispatch import candidate as internal_dispatch
    out.append(internal_dispatch(source, policy, constant))
    from data_layer_batch_seven_post_ops import candidate as signed_post_ops
    out.append(signed_post_ops(source, policy, constant))
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'packs/data-layer/candidates/rules')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for rule in generate(args.source):
        path = args.output / (rule['id'] + '.json')
        path.write_text(json.dumps(rule, indent=2) + '\n')
        print(path)


if __name__ == '__main__': main()
