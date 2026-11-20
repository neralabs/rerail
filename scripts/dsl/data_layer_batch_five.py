#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Observe fifth-batch analyzer calls while their original tests run unchanged."""
from __future__ import annotations

from dataclasses import asdict, is_dataclass
from functools import wraps
import importlib
import re
import sys
from pathlib import Path
import time

BATCH_FIVE = (
    'missing_async_pairs', 'sync_async_signatures', 'async_sync_delegation', 'public_api_triples',
    'data_layer_facade_purity', 'data_layer_compat_aliases', 'layer_violations', 'backend_coupling',
    'internal_dead_code', 'imperative_private_functions',
)
MODULES = {name: 'scripts.' + ('check_layer_violations' if name == 'layer_violations' else 'lint_' + name)
           for name in BATCH_FIVE}
TESTS = {name: 'scripts/tests/test_lint_' + name + '.py' for name in BATCH_FIVE}
TESTS['layer_violations'] = 'scripts/tests/test_check_layer_violations.py'


def normalize(check, raw, root, files, graph=None):
    if check == 'missing_async_pairs':
        missing, mismatches = raw
        raw = [dict(file=v.file, line=v.line, code='missing_pair', message=v.missing_method) for v in missing] + [
            dict(file=v.file, line=v.sync_line, code='signature_mismatch', message=v.reason) for v in mismatches]
    elif check == 'sync_async_signatures':
        raw = [dict(file=v.file, line=v.async_func.line, code='sync_async_signature_mismatch', message=str(v)) for v in raw]
    elif check == 'imperative_private_functions':
        raw = [dict(path=graph.symbols[sid].path, line=graph.symbols[sid].line, code=v.reason, message=v.detail)
               for sid, v in raw.items()]
    elif check == 'internal_dead_code':
        raw = [dict(path=graph.symbols[sid].path, line=graph.symbols[sid].line, code=category,
                    message=graph.symbols[sid].qualname) for category, ids in raw.items()
               if category.startswith('high_confidence_') for sid in ids]
    elif check == 'layer_violations':
        module = importlib.import_module(MODULES[check])
        raw = module._report_violations(raw)
    output = []
    for item in raw:
        value = asdict(item) if is_dataclass(item) else item
        path = Path(value.get('file', value.get('path', value.get('file_path', files[0]['path'] if files else 'input.py'))))
        if path.is_absolute():
            path = path.relative_to(root) if root and path.is_relative_to(root) else Path(path.name)
        code = value.get('code', value.get('issue', value.get('category')))
        if check == 'backend_coupling':code = 'BC001'
        output.append(dict(path=path.as_posix(), line=value.get('line', value.get('line_number')),
                           column=None, code=code, message=value.get('message', value.get('description', value.get('detail','')))))
    return output


def graph_files(graph):
    root = getattr(graph, 'root', None)
    if root is None:return None, []
    root = Path(root).resolve()
    files = []
    for module in getattr(graph, 'modules', {}).values():
        path = Path(module.path)
        if not path.is_absolute():path = root/path
        if path.is_file():files.append(dict(path=path.relative_to(root).as_posix(), source=path.read_text()))
    return root, sorted(files, key=lambda row:row['path'])


def install(recorder):
    modules = {name: importlib.import_module(module) for name,module in MODULES.items()}
    for name, module in modules.items():
        sys.modules.setdefault(MODULES[name].removeprefix('scripts.'), module)
    adapters = [
        ('missing_async_pairs','analyze_file','file'), ('sync_async_signatures','analyze_file','file'),
        ('async_sync_delegation','analyze_file','file'), ('async_sync_delegation','analyze_source','source'),
        ('async_sync_delegation','find_escape_hatch_violations_in_source','source'),
        ('data_layer_facade_purity','lint_file','file'), ('data_layer_facade_purity','lint_file_content','source'),
        ('data_layer_compat_aliases','lint_file','file'), ('data_layer_compat_aliases','lint_file_content','source'),
        ('public_api_triples','analyze_public_api_triples','graph'),
        ('imperative_private_functions','analyze_imperative_private_functions','graph'),
        ('internal_dead_code','analyze_dead_code','graph'),
        ('layer_violations','_check_file_violations','file'),
    ]
    def wrap(original, check, entry, kind):
        @wraps(original)
        def observed(*args, **kwargs):
            if recorder.item is None or recorder.depth:return original(*args, **kwargs)
            graph = args[0] if kind=='graph' else None
            if kind=='graph':root, files = graph_files(graph)
            else:
                supplied = Path(args[0])
                path = supplied;root = recorder.item.funcargs.get('tmp_path', path.resolve().parent)
                path = path.resolve()
                if supplied.is_absolute() and path.is_relative_to(recorder.root):root = recorder.root
                source = args[1] if kind=='source' else path.read_text() if path.is_file() else None
                logical = supplied.as_posix() if not supplied.is_absolute() else path.relative_to(root).as_posix() if path.is_relative_to(root) else path.name
                files = [dict(path=logical, source=source)] if source is not None else []
            event = dict(test=recorder.item.nodeid, check=check, entry=entry,
                         call=1+sum(e['test']==recorder.item.nodeid for e in recorder.events),
                         files=files, scope=dict(include=['**/*.py'], exclude=[]))
            if kind=='graph':event['oracle_config']={'options':{'executionOnly':kwargs.get('execution_only',True)}}
            if check=='async_sync_delegation':event['oracle_config']={'source_gate':False}
            recorder.depth += 1;started=time.perf_counter()
            try:result=original(*args, **kwargs)
            finally:recorder.depth -= 1
            event['oracle_ms']=(time.perf_counter()-started)*1000
            if not files:event['retained_reason']='Synthetic graph/helper or I/O boundary has no source corpus; not a native pass'
            else:event['findings']=normalize(check,result,root,files,graph)
            recorder.events.append(event)
            return result
        return observed
    for check, entry, kind in adapters:
        owner=modules[check];original=getattr(owner,entry)
        recorder.originals.append((owner,entry,original));setattr(owner,entry,wrap(original,check,entry,kind))
    owner=modules['backend_coupling'].BackendCouplingChecker;original=owner.check_file
    @wraps(original)
    def backend(checker, tree, path, source):
        if recorder.item is None or recorder.depth:return original(checker,tree,path,source)
        recorder.depth+=1;started=time.perf_counter()
        try:raw=original(checker,tree,path,source)
        finally:recorder.depth-=1
        root=None;path=Path(path)
        if path.is_absolute():
            for index, part in enumerate(path.parts):
                if part in ('api','impl','data_layer','_internal') or re.match(r'^l\d{4}_',part):
                    root=Path(*path.parts[:index]);break
        logical=path.relative_to(root) if root else path
        files=[dict(path=logical.as_posix(),source=source)]
        recorder.events.append(dict(test=recorder.item.nodeid,check='backend_coupling',entry='BackendCouplingChecker.check_file',
            call=1+sum(e['test']==recorder.item.nodeid for e in recorder.events),files=files,
            scope=dict(include=['**/*.py'],exclude=[]),oracle_ms=(time.perf_counter()-started)*1000,
            findings=normalize('backend_coupling',raw,root,files)))
        return raw
    recorder.originals.append((owner,'check_file',original));owner.check_file=backend
