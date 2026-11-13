"""Reconstruct passive production DTOs from native selection, facts, and report rows."""
from first_ten_production_contract import normalize
from migration import compare

CHECK='plan_scope_side_effects'


def production(backend,root):
    rows=backend.native(CHECK,'_production_contract',[],root=root)
    errors=[row['parameters']for row in rows if row['code']=='PSEP_ERROR']
    if errors:
        error=sorted(errors,key=lambda row:row['path'].split('/'))[0]
        if error['io_kind']=='InvalidUtf8':raise UnicodeDecodeError('utf-8',bytes.fromhex(error['bytes_hex']),error['start'],error['end'],error['reason'])
        raise OSError(error['errno'],error['detail'],str(root/error['path']))
    if any(row['code']not in {'PSEP_META','PSEP_FILE','PSEP_FACT','PSEP_REPORT'}for row in rows):raise ValueError('Unexpected native production response')
    meta=[row['parameters']for row in rows if row['code']=='PSEP_META']
    if len(meta)!=1:raise ValueError('One native production descriptor required')
    meta=meta[0]
    files=sorted((row['parameters']for row in rows if row['code']=='PSEP_FILE'),key=lambda p:p['path_order'])
    facts={p['path']:({meta['family']:[]}if p['present']else {})for p in files}
    for p in sorted((row['parameters']for row in rows if row['code']=='PSEP_FACT'),key=lambda p:(p['path'],p['ordinal'])):
        facts[p['path']][meta['family']].append(dict(code=p['source_code'],file_path=p['path'],line_number=p['line_number'],message=p['source_message'],severity=p['severity']))
    report=sorted((row['parameters']for row in rows if row['code']=='PSEP_REPORT'),key=lambda p:(str(root/p['path']),p['line_number'],p['source_code'],p['ordinal']))
    findings=[dict(linter_name=meta['name'],code=p['source_code'],path=str(root/p['path']),line=p['line_number'],message=p['source_message'],severity=meta['enforcement'])for p in report]
    public=backend.native(CHECK,None,[],root=root)
    actual=[dict(linter_name=meta['name'],code=row['code'],path=str(root/row['path']),line=row['line'],message=row['message'],severity=meta['enforcement'])for row in public]
    compare(actual,findings,unordered=True)
    return normalize(dict(selectedFiles=[p['path']for p in files],facts=facts,findings=findings,plugin={key:meta[key]for key in ['name','enforcement','description']}),root)
