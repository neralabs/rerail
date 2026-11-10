"""Decode complete production contract rows without host file selection."""
from first_ten_production_contract import normalize

CHECK='dynamic_module_loading'


def production(backend,root):
    rows=backend.native(CHECK,'_production_contract',[],root=root)
    known={'DL_META','DL_FILE','DL_FACT','DL_REPORT'}
    if any(r['code'] not in known for r in rows):raise ValueError('Unexpected dynamic production response')
    meta={r['parameters']['key']:r['parameters']['value'] for r in rows if r['code']=='DL_META'}
    if not meta:raise ValueError('Missing production metadata')
    files=sorted((r['parameters'] for r in rows if r['code']=='DL_FILE'),key=lambda p:p['path_order'])
    facts={p['path']:{} for p in files}
    for p in files:
        if p['fact_present']:facts[p['path']][meta['extractor']]=[]
    for row in sorted((r['parameters'] for r in rows if r['code']=='DL_FACT'),key=lambda p:(p['path_order'],p['order0'],p['order1'],p['order2'])):
        facts[row['path']][meta['extractor']].append({k.removeprefix('f_'):v for k,v in row.items() if k.startswith('f_')})
    ordered=sorted((r['parameters'] for r in rows if r['code']=='DL_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','source_order0','source_order1','source_order2']))
    findings=[dict(linter_name=meta['name'],code=p['source_code'],path=p['file'],line=p['source_line'],message=p['source_message'],severity=meta['findingSeverity']) for p in ordered]
    # The public production entry must agree too; its DTO carries full paths.
    public=backend.native(CHECK,None,[],root=root)
    public_values=[dict(linter_name=meta['name'],code=r['code'],path=r['parameters']['file'],line=r['parameters']['source_line'],message=r['parameters']['source_message'],severity=r['parameters']['severity']) for r in public]
    from migration import compare
    compare(findings,public_values,unordered=True)
    return normalize(dict(selectedFiles=[p['path'] for p in files],facts=facts,findings=findings,plugin={k:meta[k] for k in ['name','enforcement','description']}),root)
