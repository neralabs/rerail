"""Transport public plan-scope scanner arguments to the integrated native policy."""
CHECK='plan_scope_side_effects'


def install(backend,module,detectors=False):
    def scan_repo(root):
        rows=backend.native(CHECK,'_pse_repository',[],dict(env=['pse.api=repository']),root=root)
        errors=[r['parameters']for r in rows if r['code']=='PSE_READ_ERROR']
        if errors:
            error=sorted(errors,key=lambda p:p['file_path'])[0]
            if error['io_kind']=='InvalidUtf8':raise UnicodeDecodeError('utf-8',bytes.fromhex(error['bytes_hex']),error['start'],error['end'],error['reason'])
            raise OSError(error['errno'],error['detail'],str(root/error['file_path']))
        if any(r['code']!='PSE_FACT'for r in rows):raise ValueError('Unexpected native plan-scope scanner response')
        values=[r['parameters']for r in rows]
        values.sort(key=lambda p:(p['file_path'],p['line_number'],p['source_code'],p['ordinal']))
        return [module.Violation(code=p['source_code'],file_path=p['file_path'],line_number=p['line_number'],message=p['source_message'],severity=p['severity'])for p in values]
    def blocking_violations(violations):
        rows=backend.native(CHECK,'blocking_violations',[],dict(pseSeverities=[v.severity for v in violations]))
        if any(r['code']!='PSE_BLOCKING'for r in rows):raise ValueError('Unexpected native severity selection response')
        return [violations[r['parameters']['ordinal']]for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
    backend.replace(module,'scan_repo',scan_repo)
    backend.replace(module,'blocking_violations',blocking_violations)
