# SPDX-License-Identifier: Apache-2.0
"""Resolve inherited methods inside the original analyzer's admitted file scope."""
from data_layer_six.common import S


def add_inherited_methods(stages):
    imports = next(s for s in stages if s['name'] == 'call_imports')
    imports['query'] = imports['query'].replace('SELECT n.__path,own.fn,', 'SELECT n.__path,own.fn,n.kind import_kind,')
    flags = next(s for s in stages if s['name'] == 'function_flags')
    flags['indexes'].append(['__path', 'lexical_parent', 'name'])
    colored = next(s for s in stages if s['name'] == 'colored_functions')
    direct = {**colored, 'name': 'direct_colored_functions'}
    colored['query'] = colored['query'].replace('JOIN purity_call_edges ', 'JOIN complete_purity_edges ')
    additions = [direct,
      S('admitted_files', """SELECT __path FROM direct_colored_functions
        UNION SELECT __path FROM nodes WHERE kind='module' AND rx_matches(text,'pure_lazy|__pure_lazy_surface__|_lazy')
        UNION SELECT __path FROM declarations WHERE name IN ('create_embedding_ref_class','create_embedding_vector_class','register_permissions','register_table')""", ('__path',)),
      S('class_catalog', """SELECT c.__path,c.id,c.qualname,m.module||'.'||c.qualname qualified,
        row_number() OVER(ORDER BY c.__path,c.id) class_key
        FROM declaration_contexts c JOIN file_modules m ON m.__path=c.__path WHERE c.kind='class_definition'
        AND (EXISTS(SELECT 1 FROM admitted_files a WHERE a.__path=c.__path) OR NOT EXISTS(SELECT 1 FROM admitted_files))""", ('__path','id'), ('qualified',), ('class_key',)),
      S('class_bases', """SELECT child.class_key child_key,parent.class_key parent_key,base.ordinal
        FROM class_catalog child JOIN file_modules module ON module.__path=child.__path
        JOIN nodes args ON args.__path=child.__path AND args.parent=child.id AND args.kind='argument_list'
        JOIN nodes base ON base.__path=args.__path AND base.parent=args.id AND base.kind IN ('identifier','attribute')
        JOIN class_catalog parent ON parent.qualified=CASE WHEN base.kind='identifier' THEN
          coalesce((SELECT imp.target FROM call_imports imp WHERE imp.__path=child.__path AND imp.fn IS NULL AND imp.local=base.text AND imp.import_kind='import_from_statement'),module.module||'.'||base.text)
        ELSE coalesce((SELECT imp.target||'.'||base.attribute_source FROM call_imports imp WHERE imp.__path=child.__path AND imp.fn IS NULL AND imp.local=base.object_source AND imp.import_kind='import_statement'),rx_replace(base.text,'\\s+','')) END""", ('child_key','ordinal')),
      S('method_demands', """SELECT c.__path,o.fn caller,c.id call,c.callee name,cls.class_key
        FROM calls c JOIN ownership o ON o.__path=c.__path AND o.id=c.id
        JOIN function_flags f ON f.__path=o.__path AND f.id=o.fn
        JOIN class_catalog cls ON cls.__path=f.__path AND cls.id=f.lexical_parent
        LEFT JOIN nodes receiver ON receiver.__path=c.__path AND receiver.id=c.receiver
        WHERE c.__start>=f.body_start AND c.__end<=f.body_end
        AND (c.callee_kind='attribute' AND receiver.kind='identifier' AND receiver.text IN ('self','cls')
          OR c.callee_kind='identifier'
          AND NOT EXISTS(SELECT 1 FROM function_flags global WHERE global.__path=c.__path AND global.lexical_parent IS NULL AND global.name=c.callee)
          AND NOT EXISTS(SELECT 1 FROM call_imports imp WHERE imp.__path=c.__path AND imp.local=c.callee AND (imp.fn IS NULL OR imp.fn=f.id))
          AND NOT EXISTS(SELECT 1 FROM parameters p WHERE p.__path=c.__path AND p.fn=f.id AND p.name=c.callee)
          AND NOT EXISTS(SELECT 1 FROM assignments a JOIN bound_names b ON b.__path=a.__path AND b.assignment=a.id WHERE a.__path=c.__path AND a.fn=f.id AND b.name=c.callee))
        AND NOT EXISTS(SELECT 1 FROM nodes n WHERE n.__path=c.__path AND n.kind='lambda' AND n.__start<c.__start AND n.__end>=c.__end)""", ('__path','call')),
      S('inherited_edges', """WITH RECURSIVE search(__path,caller,call,name,class_key,route,seen) AS (
        SELECT __path,caller,call,name,class_key,'',','||class_key||',' FROM method_demands
        UNION ALL SELECT s.__path,s.caller,s.call,s.name,b.parent_key,s.route||printf('%08d',b.ordinal),s.seen||b.parent_key||','
        FROM search s JOIN class_bases b ON b.child_key=s.class_key
        WHERE instr(s.seen,','||b.parent_key||',')=0 AND NOT EXISTS(
          SELECT 1 FROM class_catalog cls JOIN function_flags own ON own.__path=cls.__path AND own.lexical_parent=cls.id
          WHERE cls.class_key=s.class_key AND own.name=s.name))
        SELECT __path,caller,target_path,target FROM (
          SELECT s.__path,s.caller,s.call,f.__path target_path,f.id target,
            row_number() OVER(PARTITION BY s.__path,s.call ORDER BY s.route) chosen
          FROM search s JOIN class_catalog cls ON cls.class_key=s.class_key
          JOIN function_flags f ON f.__path=cls.__path AND f.lexical_parent=cls.id AND f.name=s.name)
        WHERE chosen=1""", ('__path','caller')),
      S('complete_purity_edges', 'SELECT * FROM purity_call_edges UNION SELECT * FROM inherited_edges', ('__path','caller')),
    ]
    index = stages.index(colored)
    stages[index:index] = additions
