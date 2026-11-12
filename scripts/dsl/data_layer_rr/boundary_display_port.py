"""Transport rendering inputs, batched context reads, and native output fragments."""
from dataclasses import fields
import json
from pathlib import Path
import sys

CHECK='async_sync_boundary'
ORDER=['phase','group','section','file','part','sign','length','digits','ordinal','sub']


def query(backend,entry,files=(),options=None):return backend.native(CHECK,entry,list(files),options or {})


def emit(rows):
    for p in sorted((r['parameters'] for r in rows if r['code']=='BS_OUTPUT'),key=lambda p:tuple(p[k] for k in ORDER)):sys.stdout.write(p['text'])


def display(backend,violations,*,mode,checked=0,show_context=False,show_allowed=False,root=None):
    raw=[{f.name:getattr(v,f.name) for f in fields(v)} for v in violations]
    options=dict(bsDisplayMode=mode,bsFilesChecked=str(checked),bsShowContext=str(int(show_context)),bsShowAllowed=str(int(show_allowed)),bsHasRoot=str(int(root is not None)),bsContextRoot=str(root or ''))
    def files(contexts):return [dict(path='boundary-display.json',source=json.dumps(dict(rows=raw,contexts=contexts),ensure_ascii=False))]
    contexts=[]
    if show_context and root is not None:
        demands=query(backend,'_bs_display_demand',files([]),options)
        if any(r['code']!='BS_CONTEXT' for r in demands):raise ValueError('Unexpected boundary context demand')
        paths=dict.fromkeys(r['parameters']['path'] for r in demands)
        for path in paths:
            try:lines=Path(path).read_text(encoding='utf-8').splitlines()
            except Exception:lines=[]
            contexts.append(dict(path=path,lines=lines))
    rows=query(backend,'_bs_display',files(contexts),options)
    if any(r['code'] not in {'BS_OUTPUT','BS_DISPLAY_COUNTS'} for r in rows):raise ValueError('Unexpected native boundary output')
    emit(rows)
    if mode=='results':
        counts=[r['parameters'] for r in rows if r['code']=='BS_DISPLAY_COUNTS']
        if len(counts)!=1:raise ValueError('One boundary output count required')
        return counts[0]['actionable'],counts[0]['allowed']


def install(backend,module):
    backend.replace(module,'print_results',lambda result,show_context=False,show_allowlisted=False,root=None:display(backend,result.violations,mode='results',checked=result.files_checked,show_context=show_context,show_allowed=show_allowlisted,root=root))
    backend.replace(module,'_print_violations',lambda violations,show_context=False,root=None:display(backend,violations,mode='violations',show_context=show_context,root=root))
    def context(filepath,line,radius=2):
        try:lines=Path(filepath).read_text(encoding='utf-8').splitlines()
        except Exception:lines=[]
        rows=query(backend,'_bs_show_context',options=dict(bsContextLines=lines,bsContextLine=str(line),bsContextRadius=str(radius)))
        if any(r['code']!='BS_OUTPUT' for r in rows):raise ValueError('Unexpected standalone boundary context')
        emit(rows)
    backend.replace(module,'_show_source_context',context)
