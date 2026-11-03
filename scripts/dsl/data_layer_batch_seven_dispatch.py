# SPDX-License-Identifier: Apache-2.0
"""Post-operation dispatch boundaries and ordered literal-payload aliases."""
import re
from data_layer_six.common import A, O, N, K, F, H, I, P, optional, E, S, names, term, call, q


def candidate(source, policy, constant):
    module = source / 'scripts/lint_post_op_no_internal_dispatch.py'
    p = policy('post_op_no_internal_dispatch', ['l2070_batching/**/*.py', 'impl/common/**/*.py', 'l2060_query_builder/**/*.py', 'l5200_vector_index/**/*.py', 'l5100_fts/**/*.py', 'l6500_evidence/**/*.py'])
    materializer_name = {'kind': 'identifier', 'regex': r'^(?:materialize_result|materialize_runtime_builder|materialize_.*builder)$'}
    materializer = A(K('function_definition'), F('name', materializer_name))
    post_path = {'filePath': {'regex': '^l2070_batching/post_op\\.py$'}}
    owner = A(K('function_definition'), P('FUNCTION'), F('name', P('FUNCTION_NAME')))
    body = O(materializer, A(K('function_definition'), post_path, F('name', {'regex': '^_materialize_'})))
    in_owner = I(owner, {'kind': 'function_definition'})
    in_body = I(body, {'kind': 'function_definition'})
    enum = A(K('attribute'), P('SYMBOL'), F('object', A(names(constant(module, 'POST_OP_ENUM_NAMES')), P('ENUM'))), F('attribute', P('MEMBER')))
    left = H(A(enum, {'nthChild': 1}))
    pair = A(K('comparison_operator'), O({'pattern': '$LEFT == $RIGHT'}, {'pattern': '$LEFT is $RIGHT'}),
             O(left, A(N(left), H(A(enum, {'nthChild': 2})))), in_owner)
    def ast_clause(id, rule, code, each=None):
        p['clauses'].append({'id': id, 'forbid': {'ast': {'language': 'python', 'syntax': 'valid', 'rule': rule}},
            'violation': {'code': code, 'anchor': 'witness.span', 'message': code, **({'each': each} if each else {})}})
    def counted(id, rule, project, group, code, anchor=None):
        p['clauses'].append({'id': id, 'budget': {'query': E(rule, project, anchor=anchor), 'group_by': group, 'count': {'lte': 1}},
            'violation': {'code': code, 'anchor': 'witness.span', 'message': code}})
    counted('duplicate-enum', pair, {'path': 'file.path', 'owner': 'capture.FUNCTION.startByte', 'enum': 'capture.ENUM', 'member': 'capture.MEMBER'},
            ['row.path', 'row.owner', 'row.enum', 'row.member'], 'POSTOPDISPATCH001')
    ast_clause('builder-name', A(pair,
        I(materializer, {'kind': 'function_definition'})), 'POSTOPDISPATCH002')
    p['clauses'][-1]['forbid']['ast']['constraints'] = {'ENUM': names(['RuntimeBuilderName'])}
    literal = A(K('string'), {'regex': r'''(?i)^[ru]*["']'''}, {'value': {'regex': '(?s).*'}})
    key_pair = A(K('comparison_operator'), {'pattern': '$KEY in $CONTAINER'}, H(A(literal, {'nthChild': 1})))
    keys_call = A(call(['keys']), F('function', K('attribute')))
    builtin_keys = A(key_pair, H(A(O(names(['kwargs', 'literal_kwargs']), keys_call), {'nthChild': 2})), in_body)
    ast_clause('literal-kwargs', builtin_keys, 'POSTOPDISPATCH003')
    counted('multi-enum-builder', A(pair, I(A(K('function_definition'), F('name', {'regex': '^materialize_.*builder$'})), {'kind': 'function_definition'})),
            {'path': 'file.path', 'owner': 'capture.FUNCTION.startByte'}, ['row.path', 'row.owner'], 'POSTOPDISPATCH004', anchor='capture.FUNCTION')
    legacy = names(constant(module, 'LEGACY_RUNTIME_SURFACE_NAMES'))
    expression_name = A(legacy, N(I(O(K('import_statement'), K('import_from_statement')))),
        *[N(I(K(k), 'neighbor', field=f)) for k,f in [('attribute','attribute'),('keyword_argument','name'),('function_definition','name'),('class_definition','name'),('default_parameter','name'),('typed_default_parameter','name')]],
        N(I(O(K('parameters'), K('list_splat_pattern'), K('dictionary_splat_pattern')), 'neighbor')),
        N(A(I(K('typed_parameter'), 'neighbor'), {'nthChild': {'position': 1, 'ofRule': N(K('comment'))}})))
    ast_clause('removed-surface', O(expression_name, A(K('attribute'), F('attribute', legacy))), 'POSTOPDISPATCH005')
    ast_clause('domain-helper', A(K('function_definition'), post_path, F('name', {'regex': '^_materialize_'}),
        N(F('name', names(constant(module, 'ALLOWED_POST_OP_MATERIALIZER_HELPERS'))))), 'POSTOPDISPATCH006')
    base = A(O(names(['ResultRowMaterializer']), A(K('attribute'), F('attribute', names(['ResultRowMaterializer'])))), P('BASE'))
    ast_clause('removed-base', A(K('class_definition'), F('superclasses', H(base))), 'POSTOPDISPATCH008', 'BASE')
    keyword = A(K('keyword_argument'), P('KEYWORD'), F('name', names(['row_materializer'])))
    ast_clause('removed-keyword', A(K('call'), F('arguments', H(keyword))), 'POSTOPDISPATCH009', 'KEYWORD')

    payload = A(K('attribute'), F('attribute', names(['literal_payload'])), P('PAYLOAD'))
    assignment = A(K('assignment'), F('left', A(K('identifier'), P('TARGET'))), F('right', P('VALUE')),
        optional(F('right', O(payload, H(payload, stop='end')))))
    receiver = A(K('identifier'), P('RECEIVER'))
    get = A(K('call'), F('function', A(K('attribute'), F('object', receiver), F('attribute', names(['get'])))),
            F('arguments', H(A(literal, {'nthChild': 1}))))
    subscript = A(K('subscript'), F('value', receiver), F('subscript', literal))
    membership = A(key_pair, H(A(receiver, {'nthChild': 2})))
    key_calls = A(key_pair, H(A(keys_call, {'nthChild': 2})))
    inputs = {
        'functions': E(owner, {'name': 'capture.FUNCTION_NAME'}),
        'raw_assignments': E(assignment, {'target': 'capture.TARGET', 'value': 'capture.VALUE', 'value_kind': 'capture.VALUE.kind', 'payload': 'capture.PAYLOAD.startByte'}),
        'raw_probes': E(A(O(get, subscript, membership), in_body), {'receiver': 'capture.RECEIVER', 'membership': {'source': 'capture.RECEIVER', 'regex': '^(kwargs|literal_kwargs)$', 'group': 1}}),
        'key_calls': E(A(key_calls, in_body), {'path': 'file.path'}),
    }
    stages = [
        S('assignments', '''SELECT a.*,(SELECT f.__start FROM functions f WHERE f.__path=a.__path AND f.__start<a.__start AND f.__end>=a.__end ORDER BY f.__start DESC LIMIT 1) AS owner FROM raw_assignments a''', ('__path','owner','__start')),
        S('aliases', '''WITH RECURSIVE alias(path,owner,name,since) AS (
            SELECT __path,__start,'literal_payload',-1 FROM functions
            UNION SELECT __path,owner,target,__start FROM assignments WHERE payload IS NOT NULL
            UNION SELECT a.__path,a.owner,a.target,a.__start FROM assignments a JOIN alias old ON old.path=a.__path AND old.owner=a.owner AND old.name=a.value AND old.since<=a.__start WHERE a.value_kind='identifier')
            SELECT path,owner,name,min(since) AS since FROM alias GROUP BY path,owner,name''', ('path','owner','name','since')),
        S('probes', '''SELECT p.*,(SELECT f.__start FROM functions f WHERE f.__path=p.__path AND f.__start<p.__start AND f.__end>=p.__end ORDER BY f.__start DESC LIMIT 1) AS owner FROM raw_probes p'''),
    ]
    query = '''SELECT p.__path AS path,p.__start AS start_byte,p.__end AS end_byte FROM probes p
        WHERE EXISTS(SELECT 1 FROM aliases a WHERE a.path=p.__path AND a.owner=p.owner AND a.name=p.receiver AND a.since<=p.__start)
        UNION ALL SELECT __path,__start,__end FROM key_calls'''
    # kwargs/literal_kwargs are unconditional only for membership, not .get/index.
    # This separate matcher preserves the oracle's surprising broad membership rule.
    direct_membership = A(key_pair, H(A(names(['kwargs','literal_kwargs']), {'nthChild': 2})), in_body)
    inputs['direct_membership'] = E(direct_membership, {'path': 'file.path'})
    query += ''' UNION SELECT __path,__start,__end FROM direct_membership'''
    p['clauses'].append({'id': 'payload-aliases', 'relational': {'inputs': inputs, 'partitionBy': 'file', 'stages': stages, 'query': query, 'key': ['start_byte']},
        'violation': {'code': 'POSTOPDISPATCH007', 'anchor': 'witness.span', 'message': 'Post-op payload dispatch'}})
    triggers = set(constant(module, 'POST_OP_ENUM_NAMES')) | set(constant(module, 'LEGACY_RUNTIME_SURFACE_NAMES')) | {'_materialize_', 'literal_payload', 'ResultRowMaterializer', 'row_materializer'}
    root = A(K('module'), {'regex': '|'.join(map(re.escape, sorted(triggers)))})
    gate = O(root, I(root))
    def source_gate(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key == 'ast' and isinstance(child, dict) and 'rule' in child:
                    child['rule'] = A(child['rule'], gate)
                else: source_gate(child)
        elif isinstance(value, list):
            for child in value: source_gate(child)
    source_gate(p['clauses'])
    return p
