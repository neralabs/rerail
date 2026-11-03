# SPDX-License-Identifier: Apache-2.0
"""Merged identity ownership and source-ordered projection aliases."""
from data_layer_six.common import A, O, K, F, H, I, P, optional, E, S, q, literals, names


def candidate(source, policy, constant):
    module = source / 'scripts/check_versioned_merged_identity.py'
    p = policy('versioned_merged_identity', ['impl/common/l2070_server_plan/steps/*.py', 'impl/common/l9700_sql_graph_query/*.py'])
    p['clauses'].append({'id': 'contract', 'define': {'options': {
        'expectedCallers': {'type': 'array', 'items': 'string', 'default': sorted(constant(module, 'EXPECTED_CALLERS'))}}}})
    frame = lambda kind: A(K(kind), F('name', P('NAME')), optional(I(A(K('decorated_definition'), P('WRAPPER')), 'neighbor')))
    callee = O(A(K('identifier'), P('CALLEE')), A(K('attribute'), F('attribute', P('CALLEE'))))
    assignment = A(K('assignment'), F('right', P('VALUE')), O(
        F('left', A(K('identifier'), P('TARGET'))),
        A(F('right', K('call')), F('left', A(O(K('pattern_list'), K('list_pattern'), K('tuple_pattern')), H(A(K('identifier'), P('TARGET'))))))))
    inputs = {
        'raw_functions': E(frame('function_definition'), {'name': 'capture.NAME', 'wrapper': 'capture.WRAPPER.startByte'}),
        'raw_classes': E(frame('class_definition'), {'name': 'capture.NAME', 'wrapper': 'capture.WRAPPER.startByte'}),
        'raw_calls': E(A(K('call'), F('function', callee), optional(F('arguments', H(A(K('keyword_argument'), F('name', names(['read_projection'])), F('value', P('PROJECTION'))))))),
                       {'name': 'capture.CALLEE', 'filename': 'file.name', 'projection': 'capture.PROJECTION', 'projection_kind': 'capture.PROJECTION.kind', 'projection_start': 'capture.PROJECTION.startByte'}),
        'raw_assignments': E(assignment, {'target': 'capture.TARGET', 'value': 'capture.VALUE', 'value_kind': 'capture.VALUE.kind', 'value_start': 'capture.VALUE.startByte'}, each='TARGET'),
        'raw_aliases': E(A(K('aliased_import'), F('name', {'regex': '^SqlVersionedMergedCte$'}), F('alias', P('ALIAS')), I(K('import_from_statement'))), {'alias': 'capture.ALIAS'}),
    }
    allowed = literals(constant(module, 'ALLOWED_PROJECTION_BUILDERS'))
    stages = [
        S('frames', 'SELECT __path,__start,__end,coalesce(wrapper,__start) AS lexical_start,name,1 AS is_function FROM raw_functions UNION ALL SELECT __path,__start,__end,coalesce(wrapper,__start),name,0 FROM raw_classes', ('__path','__start')),
        S('parents', 'SELECT f.*,(SELECT p.__start FROM frames p WHERE p.__path=f.__path AND p.lexical_start<f.lexical_start AND p.__end>=f.__end ORDER BY p.lexical_start DESC LIMIT 1) AS parent FROM frames f', ('__path','__start')),
        S('contexts', '''WITH RECURSIVE scope(path,start,qualname) AS (
          SELECT __path,__start,name FROM parents WHERE parent IS NULL
          UNION ALL SELECT c.__path,c.__start,p.qualname||'.'||c.name FROM scope p JOIN parents c ON c.__path=p.path AND c.parent=p.start)
          SELECT * FROM scope''', ('path','start')),
        S('call_owners', '''SELECT c.*,(SELECT f.__start FROM frames f WHERE f.is_function AND f.__path=c.__path AND f.lexical_start<c.__start AND f.__end>=c.__end ORDER BY f.lexical_start DESC LIMIT 1) AS owner,
          coalesce((SELECT x.qualname FROM frames f JOIN contexts x ON x.path=f.__path AND x.start=f.__start WHERE f.__path=c.__path AND f.lexical_start<c.__start AND f.__end>=c.__end ORDER BY f.lexical_start DESC LIMIT 1),'') AS qualname FROM raw_calls c''', ('__path','__start')),
        S('constructors', "SELECT c.*,filename||':'||qualname AS caller FROM call_owners c WHERE name='SqlVersionedMergedCte' OR EXISTS(SELECT 1 FROM raw_aliases a WHERE a.__path=c.__path AND a.alias=c.name)", ('__path','__start')),
        S('assignments', '''SELECT a.*,(SELECT f.__start FROM frames f WHERE f.is_function AND f.__path=a.__path AND f.lexical_start<a.__start AND f.__end>=a.__end ORDER BY f.lexical_start DESC LIMIT 1) AS owner FROM raw_assignments a''', ('__path','owner','target','__end')),
        S('versions', '''SELECT c.__path,c.__start AS site,a.target,a.value,a.value_kind,a.value_start FROM constructors c JOIN assignments a ON a.__path=c.__path AND a.owner=c.owner AND a.__end<c.__start
          WHERE NOT EXISTS(SELECT 1 FROM assignments later WHERE later.__path=a.__path AND later.owner=a.owner AND later.target=a.target AND later.__end>a.__end AND later.__end<c.__start)''', ('__path','site','target')),
        S('catalog_aliases', f'''WITH RECURSIVE catalog(path,site,name) AS (
          SELECT v.__path,v.site,v.target FROM versions v JOIN raw_calls c ON c.__path=v.__path AND c.__start=v.value_start WHERE v.value_kind='call' AND c.name IN {allowed}
          UNION SELECT v.__path,v.site,v.target FROM versions v JOIN catalog c ON c.path=v.__path AND c.site=v.site AND c.name=v.value WHERE v.value_kind='identifier')
          SELECT * FROM catalog''', ('path','site','name')),
        S('expected', {'$option': 'expectedCallers', 'map': " UNION ALL SELECT '{}' AS caller", 'join': '', 'template': "SELECT '' AS caller WHERE 0{}"}, ('caller',)),
    ]
    query = f'''SELECT c.__path AS path,c.__start AS start_byte,c.__end AS end_byte,c.caller,'VMI002' AS issue FROM constructors c WHERE NOT EXISTS(SELECT 1 FROM expected e WHERE e.caller=c.caller)
      UNION ALL SELECT c.__path,c.__start,c.__end,c.caller,'VMI003' FROM constructors c WHERE EXISTS(SELECT 1 FROM expected e WHERE e.caller=c.caller)
        AND NOT ((c.projection_kind='call' AND EXISTS(SELECT 1 FROM raw_calls p WHERE p.__path=c.__path AND p.__start=c.projection_start AND p.name IN {allowed}))
          OR (c.projection_kind='identifier' AND EXISTS(SELECT 1 FROM catalog_aliases a WHERE a.path=c.__path AND a.site=c.__start AND a.name=c.projection)))
      UNION ALL SELECT 'impl/common/l2070_server_plan/steps',0,0,e.caller,'VMI001' FROM expected e WHERE NOT EXISTS(SELECT 1 FROM constructors c WHERE c.caller=e.caller)'''
    # coalesce makes a missing projection a failing contract, not SQL NULL.
    query = query.replace('AND NOT ((c.projection_kind', 'AND NOT coalesce(((c.projection_kind').replace('a.name=c.projection)))', 'a.name=c.projection))),0)')
    p['clauses'].append({'id': 'constructors', 'relational': {'inputs': inputs, 'stages': stages, 'query': query,
        'locations': ['impl/common/l2070_server_plan/steps'], 'key': ['caller', 'issue'], 'code': 'issue'},
        'violation': {'code': 'VMI003', 'anchor': 'witness.span', 'message': 'Merged identity constructor contract'}})
    return p
