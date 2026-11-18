# SPDX-License-Identifier: Apache-2.0
"""Read literal approval configuration without importing or executing the linter."""
import ast
import json
from pathlib import Path
import sys

path = Path(sys.argv[1]) / 'scripts/lint_schema_introspection_data_paths.py'
values = {}
for node in ast.parse(path.read_text(encoding='utf-8')).body:
    targets = node.targets if isinstance(node, ast.Assign) else [node.target] if isinstance(node, ast.AnnAssign) else []
    for target in targets:
        if not isinstance(target, ast.Name) or target.id not in {'APPROVED_USER_SCHEMA_CHECK_CALLSITES', 'APPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE'}:
            continue
        value = node.value
        values[target.id] = value.id if isinstance(value, ast.Name) and value.id in {'NEEDS_SIGNING', 'UNSIGNED'} else ast.literal_eval(value)
if set(values) != {'APPROVED_USER_SCHEMA_CHECK_CALLSITES', 'APPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE'}:
    raise ValueError('Schema approval configuration is incomplete')
print(json.dumps(dict(entries=values['APPROVED_USER_SCHEMA_CHECK_CALLSITES'], signature=values['APPROVED_USER_SCHEMA_CHECK_CALLSITES_SIGNATURE'])))
