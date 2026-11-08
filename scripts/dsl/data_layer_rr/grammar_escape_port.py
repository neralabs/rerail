"""Transport grammar escape-hatch arguments and DTOs; rule decisions belong to native relations."""

GROUPS = ['ALLOWLISTED_CLASSES','DATA_VALUE_PARAMS','OPERATOR_PARAMS','_SAFE_PARAM_TYPES_STATIC',
          'SAFE_PARAM_TYPES','TRANSPARENT_CONTAINER_TYPES','SCHEMA_METADATA_CLASSES',
          'CALLSITE_PROTECTED_CLASSES','FORBIDDEN_ESCAPE_HATCHES']


def install(suite,module):
    rows=suite.native('sql_grammar_escape_hatches','configuration',[])
    for group in GROUPS:
        suite.replace(module,group,frozenset(r['parameters']['value'] for r in rows
                      if r['code']=='SGE_CONSTANT' and r['parameters']['family']==group))
    for row in rows:
        if row['code']=='SGE_METADATA':
            key,value=row['parameters']['key'],row['parameters']['value']
            suite.replace(module,key,int(value) if key=='MAX_ESCAPE_HATCH_VIOLATIONS' else value)
