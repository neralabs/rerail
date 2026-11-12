"""Transport supplied structural trees and file content; classify them in native rules."""
import json
from ast_transport import columns

CHECK='connection_structural_boundaries'


def install(backend,module):
    rows=backend.native(CHECK,'_structural_helper_configuration',[])
    if not rows or any(r['code']!='CSB_SETTING'for r in rows):raise ValueError('Missing native structural helper configuration')
    for row in rows:
        p=row['parameters'];value=json.loads(p['value'])
        backend.replace(module,p['key'],frozenset(value) if isinstance(value,list) else value)
    def helper(trees):
        path=module.CONNECTION_FILE;tree=trees[path]
        source=path.read_text(encoding='utf-8')
        options=dict(columns(tree),schMaxLines=str(module.CONNECTION_FILE_MAX_LINES),
            schResidualSymbols=sorted(module.CONNECTION_RESIDUAL_SYMBOLS),schAllowedBases=sorted(module.ALLOWED_DATABASE_CORE_BASES))
        rows=backend.native(CHECK,'_connection_file_violations',[dict(path='_structural_helper.py',source=source)],options)
        if any(r['code']!='CSB_HELPER'for r in rows):raise ValueError('Unexpected native structural helper finding')
        return [module.StructuralViolation(p['source_code'],path,p['line_number'],p['source_message'])for r in sorted(rows,key=lambda r:(r['parameters']['phase'],r['parameters']['ordinal']))for p in [r['parameters']]]
    backend.replace(module,'_connection_file_violations',helper)
