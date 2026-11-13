"""Transport public plan-scope scanner arguments to the integrated native policy."""
from pathlib import Path
CHECK='plan_scope_side_effects'


def install(backend,module,detectors=False):
    def violations(root,entry,options):
        rows=backend.native(CHECK,entry,[],options,root=root.absolute())
        errors=[r['parameters']for r in rows if r['code']=='PSE_READ_ERROR']
        if errors:
            error=sorted(errors,key=lambda p:p['order_key'])[0]
            if error['io_kind']=='InvalidUtf8':raise UnicodeDecodeError('utf-8',bytes.fromhex(error['bytes_hex']),error['start'],error['end'],error['reason'])
            raise OSError(error['errno'],error['detail'],str(root/error['file_path']))
        if any(r['code']!='PSE_FACT'for r in rows):raise ValueError('Unexpected native plan-scope scanner response')
        values=[r['parameters']for r in rows]
        values.sort(key=lambda p:(p['file_path'],p['line_number'],p['source_code'],p['ordinal']))
        return [module.Violation(code=p['source_code'],file_path=p['file_path'],line_number=p['line_number'],message=p['source_message'],severity=p['severity'])for p in values]
    def scan_repo(root):
        return violations(root,'_pse_repository',dict(env=['pse.api=repository']))
    def explicit(root,files):
        # Caller-selected resources authorize their parent directories. Native
        # relations decide suffixes, file kinds, aliases, errors and findings.
        parents=list(dict.fromkeys((p if p.is_absolute()else root/p).absolute().parent.as_posix()for p in files))
        options=dict(env=['pse.api=files',f'pse.physicalRoot={root.absolute().as_posix()}',f'pse.ownerRoot={root.as_posix()}',*[f'pse.explicit.{i}={p}'for i,p in enumerate(parents)],*[f'pse.file.{i}={p.as_posix()}'for i,p in enumerate(files)]])
        return violations(root,'_pse_files',options)
    def scan_file_content(rel_path,source,tree=None):
        return explicit(module.REPO_ROOT,[module.REPO_ROOT/rel_path])
    def iter_python_files(root):
        rows=backend.native(CHECK,'_pse_iter',[],dict(env=['pse.api=iter']),root=root.absolute())
        if any(r['code']!='PSE_ITER'for r in rows):raise ValueError('Unexpected native path iteration response')
        return [root/r['parameters']['path']for r in sorted(rows,key=lambda r:r['parameters']['order_key'])]
    def blocking_violations(violations):
        rows=backend.native(CHECK,'blocking_violations',[],dict(pseSeverities=[v.severity for v in violations]))
        if any(r['code']!='PSE_BLOCKING'for r in rows):raise ValueError('Unexpected native severity selection response')
        return [violations[r['parameters']['ordinal']]for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
    backend.replace(module,'scan_repo',scan_repo)
    backend.replace(module,'_iter_python_files',iter_python_files)
    backend.replace(module,'_run_fact_engine_for_files',explicit)
    backend.replace(module,'scan_file_content',scan_file_content)
    backend.replace(module,'blocking_violations',blocking_violations)
