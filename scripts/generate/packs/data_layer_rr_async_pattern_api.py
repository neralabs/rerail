"""Export pinned async-pattern metadata for native adapters and registered collectors."""
from dataclasses import asdict
import json
from pathlib import Path
from data_layer_rr_text import module

CONSTANTS=['VIOLATIONS','SYNC_WRITE_METHODS','SYNC_READ_METHODS','SYNC_INTERNAL_IO_METHODS','SYNC_CONNECTION_METHODS','SYNC_POLYMORPHIC_METADATA_METHODS','ALL_SYNC_TRANSACTION_METHODS','SCAN_PATHS','EXCLUDE_PATTERNS','TRANSACTION_PARAM_NAMES','TRANSACTION_TYPE_NAMES','QUERY_SOURCE_METHODS','SOURCE_TRIGGERS']


def suffix(source):
    original=module(source,'async_patterns');plugin=original.async_patterns_plugin(Path('.'));scope=asdict(plugin.file_scope)
    scope['directories']=[str(p) for p in scope['directories']];scope['explicit_files']=[str(p) for p in scope['explicit_files']]
    values={name:dict(kind=type(getattr(original,name)).__name__,value=sorted(getattr(original,name)) if isinstance(getattr(original,name),frozenset) else getattr(original,name)) for name in CONSTANTS}
    initial=vars(original.AsyncPatternCollector(Path('file.py'),'',Path('.'))).copy()
    for name in ['filepath','root','source_lines','excluded','in_scope','in_impl','in_api']:initial.pop(name)
    config=dict(collector_defaults=initial,constants=values,extractor=asdict(original.ASYNC_PATTERNS_EXTRACTOR),scope=scope,plugin=dict(name=plugin.name,enforcement=plugin.enforcement,description=plugin.description))
    return ''.join('ap_config('+json.dumps(k)+','+json.dumps(json.dumps(v))+').\n' for k,v in config.items())+'#include "data-layer/async-pattern-api.rr"\n#include "data-layer/async-pattern-helpers.rr"\n'
