# SPDX-License-Identifier: Apache-2.0
"""Whole-file syntax demand with narrow SQL relations and native ownership facts."""
import re
from data_layer_six.ast_relations import relational,python_stages,trim_ast_projection,whole_file_syntax_relations
from data_layer_six.sql_columns import narrow_columns

def clause(name,stages,query,inputs=None):
    item=relational(name,[*python_stages(),*stages],query)
    plan=item['relational'];plan['partitionBy']='file'
    plan['inputs'].update(inputs or {})
    # Retain only actual consumers; do not publish generic unused relations.
    catalog={s['name']:s for s in plan['stages']};needed=set();pending=[query]
    while pending:
        for table in re.findall(r'(?i)\b(?:FROM|JOIN)\s+([a-z_]\w*)',pending.pop()):
            if table in catalog and table not in needed:
                needed.add(table);pending.append(catalog[table]['query'])
    plan['stages']=[s for s in plan['stages'] if s['name'] in needed]
    trim_ast_projection(item);whole_file_syntax_relations(item);plan['requireNonemptyInput']='syntax_gate'
    narrow_columns(plan)
    return item
