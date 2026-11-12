"""Transport boundary production contracts and verify the default finding set."""
from first_ten_production_contract import normalize
from migration import compare

CHECK='async_sync_boundary'


def production(backend, root, *, check=CHECK):
    rows=backend.native(check,'_production_contract',[],root=root)
    if any(r['code'] not in {'DL_META','DL_FILE','DL_FACT','DL_REPORT'} for r in rows):
        raise ValueError(f'Unexpected {check} production response')
    meta={r['parameters']['key']:r['parameters']['value'] for r in rows if r['code']=='DL_META'}
    if not meta:raise ValueError(f'Missing {check} production metadata')
    files=sorted((r['parameters'] for r in rows if r['code']=='DL_FILE'),key=lambda p:p['path_order'])
    facts={p['path']:({meta['extractor']:[]} if p['fact_present'] else {}) for p in files}
    for p in sorted((r['parameters'] for r in rows if r['code']=='DL_FACT'),key=lambda p:tuple(p[k] for k in ['path_order','order0','order1','order2'])):
        row={k[2:]:v for k,v in p.items() if k.startswith('f_')}
        compare(row['is_async_ctx'] in [0,1],True)
        row['is_async_ctx']=bool(row['is_async_ctx'])
        facts[p['path']][meta['extractor']].append(row)
    ordered=sorted((r['parameters'] for r in rows if r['code']=='DL_REPORT'),key=lambda p:tuple(p[k] for k in ['order0','order1','order2','order3','source_order0','source_order1','source_order2']))
    findings=[dict(linter_name=meta['name'],code=p['source_code'],path=p['file'],line=p['source_line'],message=p['source_message'],severity=meta['findingSeverity']) for p in ordered]
    public=backend.native(check,None,[],root=root)
    actual=[dict(linter_name=meta['name'],code=r['code'],
                 path=r['path'] if meta.get('relative_reports')=='true' else str(root/r['path']),
                 line=r['line'],message=r['message'],severity=meta['findingSeverity']) for r in public]
    compare(findings,actual,unordered=True)
    return normalize(dict(selectedFiles=[p['path'] for p in files],facts=facts,findings=findings,
                          plugin={k:meta[k] for k in ['name','enforcement','description']}),root)
