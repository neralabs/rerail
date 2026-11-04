# SPDX-License-Identifier: Apache-2.0
"""Transport standalone semantic helper requests to shared native rule relations."""
import json
from dataclasses import asdict
from pathlib import Path

from data_layer_batch_eight import tree_source
from native_scanners import logical


def install(backend):
    eager=backend.modules['eager_resolve']
    def metadata(**parameters):
        rows=backend.execute('eager_resolve','metadata',[],contract='eager-metadata',parameters=parameters)
        if len(rows)!=1: raise AssertionError('Native eager metadata must return exactly one result')
        return rows[0]['variables']['row']
    def verify(entries):
        return json.loads(metadata(entries=[asdict(item) for item in entries])['entries'])
    backend.replace(eager,'_is_valid_external_io_boundary_allowlist_entry',lambda entry:bool(verify([entry])[0]['valid']))
    backend.replace(eager,'_load_external_io_boundary_allowlist',lambda:sorted(item['symbol'] for item in verify(eager._load_external_io_boundary_allowlist_entries()) if item['valid']))
    def fabrication():
        return [eager.Violation(file_path=eager.EXTERNAL_IO_BOUNDARY_ALLOWLIST_FILE,line=item['line'],symbol=item['symbol'],
          pattern_name='fabricated-allowlist-entry:external-io-boundary',
          text='Unsigned or invalid external-I/O boundary allowlist entry. Remove this entry and fix the underlying eager functionality; see '+eager.EXTERNAL_IO_BOUNDARY_REMEDIATION_DOC+'.')
          for item in verify(eager._load_external_io_boundary_allowlist_entries()) if not item['valid']]
    backend.replace(eager,'_external_io_boundary_allowlist_fabrication_violations',fabrication)
    backend.replace(eager,'_should_ignore_pattern_violation',lambda *,file_path,text,pattern_name,symbol:bool(metadata(
        path=logical(file_path,eager.ROOT),text=text,pattern=pattern_name,symbol=symbol,approved=eager._load_external_io_boundary_allowlist())['ignored']))
    def function_row(check,contract,node):
        source=tree_source(node,backend.trees)
        if source is None: raise AssertionError('Native helper requires the actual AST source')
        rows=backend.execute(check,'function-fact',[dict(path='input.py',source=source)],contract=contract)
        values=[json.loads(row['variables']['row']['payload']) for row in rows]
        matches=[value for value in values if value['name']==node.name and value['line']==node.lineno]
        if len(matches)!=1: raise AssertionError('Native helper must identify exactly one original function')
        return matches[0]
    backend.replace(eager,'_extract_eager_lazy_wrapper_signature',lambda node:function_row('eager_resolve','eager-facts',node)['family'])
    adapter=backend.modules['lazy_adapter_ownership']
    def fact(node,*,class_name):
        value=function_row('lazy_adapter_ownership','adapter-facts',node)
        value['class_name']=class_name
        for name in ('is_async','adapter_stub','abstract_stub','has_await'): value[name]=bool(value[name])
        return value
    backend.replace(adapter,'_function_fact',fact)
