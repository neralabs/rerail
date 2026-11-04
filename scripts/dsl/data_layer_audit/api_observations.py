# SPDX-License-Identifier: Apache-2.0
"""Record complete public API results; no expected value enters native execution."""
from dataclasses import asdict,is_dataclass
from functools import wraps
from pathlib import Path
import threading

from native_scanners import logical

SCANNERS={
 'eager_resolve':['_scan_source','_scan_file','_scan_preprocess_ast','_scan_lower_compile_ast','_scan_transitive_eager_ast','_scan_legacy_boundary_definition_ast','_scan_external_io_boundary_definition_ast'],
 'lazy_architecture':['scan_files','_scan_analyzed_files'],
 'pure_lazy_runtime_purity':['scan_source','scan_paths','scan_paths_cached'],
 'cached_pure_lazy_allowlist':['policy_violations','memoized_policy_violations','scan_cached_pure_lazy_source','discover_cached_pure_lazy_functions','discover_memoized_pure_lazy_functions','cached_pure_lazy_runtime_source_hash'],
 'plan_helpers_allowlist':['scan_file_content','scan_file','scan_repo','_violations_for_files'],
 'legacy_callback_quarantine':['scan_file_content','scan_repo'],
 'lazy_sql_surface':['scan_source','scan_file','scan_repo'],
 'readthrough_authority_architecture':['scan_sources'],
 'authentication_plan_purity':['scan_sources'],
 'lazy_adapter_ownership':['scan_source','violations'],
}


def install(backend):
    backend.api_observations=[];state=threading.local()
    for check,names in SCANNERS.items():
        module=backend.modules[check]
        for entry in names:
            original=getattr(module,entry)
            def wrap(original,check,entry,module):
                @wraps(original)
                def observed(*args,**kwargs):
                    if getattr(state,'depth',0):return original(*args,**kwargs)
                    root=Path(getattr(module,'ROOT',getattr(module,'REPO_ROOT',backend.root)))
                    def serialize(value):
                        if is_dataclass(value):return serialize(asdict(value))
                        if isinstance(value,Path):
                            path=logical(value,root)
                            parts=Path(path).parts
                            if len(parts)>1 and parts[0]=='tmp':
                                for index,part in enumerate(parts):
                                    if part in ('api','impl','data_layer','_internal') or len(part)>5 and part[0]=='l' and part[1:5].isdigit() and part[5]=='_':
                                        return Path(*parts[index:]).as_posix()
                            return path
                        if isinstance(value,dict):return {key:serialize(part) for key,part in value.items()}
                        if isinstance(value,(list,tuple)):return [serialize(part) for part in value]
                        return value
                    row=dict(test=backend.item.nodeid if backend.item else '<setup>',check=check,entry=entry)
                    state.depth=1
                    try:
                        result=original(*args,**kwargs);row['result']=serialize(result);return result
                    except Exception as error:
                        row['exception']=dict(type=type(error).__name__,message=str(error))
                        if isinstance(error,SyntaxError):row['exception']['attributes']={name:serialize(getattr(error,name)) for name in ('filename','msg','lineno','offset','text','end_lineno','end_offset')}
                        raise
                    finally:
                        state.depth=0;backend.api_observations.append(row)
                return observed
            backend.replace(module,entry,wrap(original,check,entry,module))
