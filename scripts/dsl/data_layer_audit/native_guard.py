# SPDX-License-Identifier: Apache-2.0
"""Reject upstream detection; retain explicit host orchestration and passive DTOs."""
import inspect
import sys
import threading
from dataclasses import is_dataclass
from pathlib import Path


HOST = {
    'eager_resolve': {'_load_external_io_boundary_allowlist_entries','_preflight_current_allowlist_signing','_sign_current_allowlist','_strict_allowlist_entry'},
    'lazy_architecture': {'scan','_repo_files','_parse_file','_lazy_architecture_scope_dirs','lazy_architecture_plugin','lazy_architecture_fact_extractors',
      '_fact_view_to_lazy_architecture_inputs','_function_facts_from_fact','_function_facts_to_fact','_violation_from_fact','_violation_to_fact',
      '_load_allowlist','_parse_allowlist_text','_load_allowlist_budget','_tuple2_items','_tuple3_items','_tuple3_string_items','_dedupe_violations','_budget_regressions','cli_main'},
    'plan_helpers_allowlist': {'PlanHelperDetector.__init__','_TargetHelperSet.__contains__','cli_main'},
    'pure_lazy_runtime_purity': {'_preflight_current_allowlist_signing','_strict_allowlist_entry'},
}


def codes(modules):
    result={}
    for check,module in modules.items():
        for value in vars(module).values():
            functions=vars(value).values() if inspect.isclass(value) and value.__module__==module.__name__ else [value]
            for function in functions:
                if isinstance(function,(staticmethod,classmethod)):function=function.__func__
                if isinstance(function,property):function=function.fget
                if inspect.isfunction(function) and function.__module__==module.__name__:
                    # Dataclass collectors also contain detector code. Only generated storage
                    # methods are passive; source-defined methods must pass the semantic guard.
                    storage = inspect.isclass(value) and (is_dataclass(value) or
                        (issubclass(value, tuple) and isinstance(getattr(value, '_fields', None), tuple)))
                    passive = storage and function.__name__.startswith('__') and function.__code__.co_filename == '<string>'
                    result[function.__code__]=(check,function.__qualname__,passive)
    return result


def install(backend,original_codes):
    backend.original_calls={};backend.semantic_fallbacks=[]
    def audit(frame,event,arg):
        if event!='call' or frame.f_code not in original_codes:return
        check,name,passive=original_codes[frame.f_code]
        key=check+':'+name
        backend.original_calls[key]=backend.original_calls.get(key,0)+1
        if not passive and name not in HOST.get(check,set()):
            backend.semantic_fallbacks.append(dict(test=backend.item.nodeid if backend.item else '<setup>',function=key))
            raise AssertionError('Original Python semantic execution is forbidden: '+key)
    sys.setprofile(audit);threading.setprofile(audit)
