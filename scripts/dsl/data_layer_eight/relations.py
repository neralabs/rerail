# SPDX-License-Identifier: Apache-2.0
"""Whole-file syntax demand with narrow SQL relations and native ownership facts."""
import json,re
from data_layer_six.ast_relations import json,relational,python_stages,trim_ast_projection,whole_file_syntax_relations
from data_layer_six.sql_columns import narrow_columns

def clause(name,stages,query,inputs=None):
    expressions=python_stages()
    for stage in expressions:
        if stage['name']=='parameters':
            stage['query']=stage['query'].replace("(n.id=p.id AND p.kind='identifier' OR n.parent=p.id AND n.ordinal=0 AND n.kind='identifier')", "n.id=CASE WHEN p.kind='identifier' THEN p.id ELSE (SELECT child.id FROM nodes child WHERE child.__path=p.__path AND child.parent=p.id AND child.ordinal=0 AND child.kind='identifier' LIMIT 1) END")
        elif stage['name']=='arguments':
            stage['query']=stage['query'].replace("(n.parent=c.arguments_kind||':'||c.arguments_start||':'||c.arguments_end AND c.arguments_kind!='generator_expression' OR n.id=c.arguments_kind||':'||c.arguments_start||':'||c.arguments_end AND c.arguments_kind='generator_expression')", "n.parent=CASE WHEN c.arguments_kind='generator_expression' THEN c.id ELSE c.arguments_kind||':'||c.arguments_start||':'||c.arguments_end END AND (c.arguments_kind!='generator_expression' OR n.id=c.arguments_kind||':'||c.arguments_start||':'||c.arguments_end)")
    item=relational(name,[*expressions,*stages],query)
    plan=item['relational'];plan['partitionBy']='file'
    plan['inputs'].update(inputs or {})
    # Retain only actual consumers; do not publish generic unused relations.
    catalog={s['name']:s for s in plan['stages']};needed=set();pending=[query]
    while pending:
        for table in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',json.dumps(pending.pop())):
            if table in catalog and table not in needed:
                needed.add(table);pending.append(catalog[table]['query'])
    plan['stages']=[s for s in plan['stages'] if s['name'] in needed]
    trim_ast_projection(item);whole_file_syntax_relations(item);plan['requireNonemptyInput']='syntax_gate'
    narrow_columns(plan)
    return item
