"""Transport raw arguments to the native bypass rule; return passive source DTOs."""
from pathlib import Path


def install(suite,module):
    def native(entry,documents=(),options=None):
        raw=dict(allowlistEntries=[entry for entries in module.ALLOWLISTED_CALLSITES.values() for entry in entries],
                 publicKey=module.ALLOWLIST_PUBLIC_KEY_HEX,signature=module.ALLOWLIST_SIGNATURE)
        raw.update(options or {})
        return suite.native('serverplan_bypass',entry,list(documents),raw)
    configuration=native('configuration')
    suite.replace(module,'BYPASS_METHODS',frozenset(r['parameters']['value'] for r in configuration if r['code']=='SB_CONFIG'))
    def allowlisted():
        return frozenset(r['parameters']['value'] for r in native('_get_all_allowlisted') if r['code']=='SB_VALUE')
    suite.replace(module,'_get_all_allowlisted',allowlisted)
    def verify():
        rows=[r['parameters']['value'] for r in native('_verify_allowlist_integrity') if r['code']=='SB_BOOL']
        if len(rows)!=1:raise ValueError('One native signature Boolean required')
        return bool(rows[0])
    suite.replace(module,'_verify_allowlist_integrity',verify)
    def scan(path):
        path=Path(path);source=path.read_text()
        rows=native('scan_file',[dict(path='input.py',source=source)],dict(argumentPath=str(path)))
        return [module.BypassViolation(**{k:v for k,v in row['parameters'].items() if k!='ordinal'})
                for row in sorted(rows,key=lambda r:r['parameters']['ordinal']) if row['code']=='SB_ROW']
    suite.replace(module,'scan_file',scan)

    def scan_source(path,source):
        rows=[r['parameters']['value'] for r in native('_source_gate',options=dict(callSource=source)) if r['code']=='SB_BOOL']
        if len(rows)!=1:raise ValueError('One native source gate required')
        if not rows[0] or not Path(path).is_file():return []
        return scan(path)
    suite.replace(module,'scan_source',scan_source)
