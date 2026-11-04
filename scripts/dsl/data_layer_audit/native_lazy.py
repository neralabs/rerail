# SPDX-License-Identifier: Apache-2.0
"""Expose native function facts to the unchanged analyzer assertions."""
import ast
import json
from pathlib import Path

from data_layer_batch_eight import tree_source
from native_scanners import logical


EVENT_FIELDS = {
    'plan':'plan_io_calls','grammar':'grammar_calls','intent':'intent_calls','native':'imperative_native_calls',
    'dolt':'dolt_strings','context':'execution_context_terminals','internal':'lazy_internal_reads',
    'state':'execution_state_writes','private':'private_runtime_reachthroughs','forbidden_pure':'forbidden_pure_lazy_operations',
    'payload':'serverplan_service_callable_payloads','dynamic':'serverplan_dynamic_behavior_dispatches',
    'of_return':'lazy_result_of_returns','bootstrap':'bootstrap_lazy_state_mutations',
    'reconciliation':'reconciliation_eager_surface_calls','absence':'schema_fact_absence_fallbacks',
}


def install(backend):
    module = backend.modules['lazy_architecture']
    cls = module.FunctionAnalyzer
    def initialize(self, path, tree):
        source = tree_source(tree, backend.trees)
        if source is None: raise AssertionError(f'Native analyzer needs the actual original source for {path}')
        backend.fact_sources[Path(path)] = source
        self.module=tree
        name=logical(path,module.ROOT)
        rows = backend.execute('lazy_architecture','FunctionAnalyzer',[dict(path=name,source=source)],contract='lazy-facts')
        populate(self,path,[row['variables']['row'] for row in rows])
    def populate(self,path,records):
        self.file=path
        self.module_name=logical(path,module.ROOT)[:-3].replace('/','.')
        self.imported_names, self.imported_modules, self.module_rhs_bindings, self.class_method_aliases = {}, {}, {}, {}
        self.facts, by_id = [], {}
        for row in records:
            if row['relation'] != 'function': continue
            values = json.loads(row['payload']); identity = values.pop('id'); values['scope'] = tuple(values['scope'].split('.')) if values['scope'] else ()
            for field in ('is_async','is_lazy','has_pure_lazy_decorator','has_legacy_boundary_decorator','is_public','is_tombstone','is_execution_context_method'):
                values[field] = bool(values[field])
            fact = module.FunctionFacts(file=path, **values)
            self.facts.append(fact); by_id[identity] = fact
        for row in records:
            kind = row['relation']; value = json.loads(row['payload'])
            if kind == 'event':
                fact = by_id[value['fn']]; field = EVENT_FIELDS[value['category']]
                item = value['line'] if field == 'lazy_result_of_returns' else (value['line'], value['detail'])
                if field == 'plan_io_calls': item = (*item, bool(value['uses_conn']))
                if field == 'reconciliation_eager_surface_calls': item = (*item, value['event_kind'])
                getattr(fact, field).append(item)
            elif kind in ('callee','delegate'): getattr(by_id[value['fn']],'local_callee_calls' if kind=='callee' else 'deferred_lazy_delegate_calls').append((value['line'], value['target']))
            elif kind == 'import':
                if value['kind'] == 'import_from_statement': self.imported_names[value['local']] = value['module']+'.'+value['name']
                else: self.imported_modules[value['local']] = value['name']
            elif kind == 'binding': self.module_rhs_bindings[value['name']] = ast.parse(value['source'],mode='eval').body
            elif kind == 'class_alias': self.class_method_aliases[(self.module_name,value['class'],value['name'])] = value['target']
    backend.replace(cls,'__init__',initialize)
    from scripts.lint_lib.fact_engine import FactEngine
    from scripts.lint_lib.linter_plugin import PerFileFactBundle,ProjectFactView
    from native_scanners import dto
    from types import SimpleNamespace
    import hashlib
    def materialize(engine,plugins,*,max_workers=None):
        plan=engine.plan(plugins)
        expected=module.LAZY_ARCHITECTURE_EXTRACTOR.extractor_id
        if {spec.extractor_id for spec in plan.required_facts}!={expected}:
            raise AssertionError('Native materialization requires an implemented extractor contract')
        files=[];paths={}
        for path in plan.files:
            source=path.read_text(encoding='utf-8');backend.fact_sources[path]=source
            name=logical(path,engine.root);paths[name]=path;files.append(dict(path=name,source=source))
        rows=backend.execute('lazy_architecture','materialize',files,contract='lazy-facts')
        extras=backend.execute('lazy_architecture','collector-extra',files)
        bundles={}
        for file in files:
            path=paths[file['path']];analyzer=SimpleNamespace()
            populate(analyzer,path,[row['variables']['row'] for row in rows if row['path']==file['path']])
            violations=[dto('lazy_architecture',module,row,files,paths,engine.root) for row in extras if row['path']==file['path']]
            bundle=dict(module_name=analyzer.module_name,functions=[module._function_facts_to_fact(fact) for fact in analyzer.facts],extra_violations=[module._violation_to_fact(row) for row in violations])
            bundles[path]=PerFileFactBundle(path=path,content_hash=hashlib.sha256(file['source'].encode()).hexdigest(),facts={expected:bundle})
        return ProjectFactView(root=engine.root,files=bundles,required_facts=plan.required_facts)
    backend.replace(FactEngine,'materialize',materialize)
    def graph(kind,parameters):
        rows=backend.execute('lazy_architecture',kind,[],contract='lazy-graph',parameters=parameters)
        if len(rows)!=1:raise AssertionError('Native graph contract must return exactly one result')
        return rows[0]['variables']['row']
    def resolve(analyzer,facts_by_symbol):
        encode=lambda f:dict(symbol=f.symbol,name=f.name,scope=list(f.scope),calls=f.local_callee_calls)
        value=graph('targets',dict(mode='targets',module=analyzer.module_name,local=[encode(f) for f in analyzer.facts],global_=[dict(symbol=f.symbol) for f in facts_by_symbol.values()]))
        return {key:[tuple(call) for call in calls] for key,calls in json.loads(value['targets']).items()}
    backend.replace(module,'_resolve_callable_targets',resolve)
    def transitive(facts_by_symbol,callable_targets,symbol,target_symbols,memo,visiting):
        value=graph('reach',dict(mode='reach',root=symbol,facts=list(facts_by_symbol),calls=callable_targets,targets=sorted(target_symbols),memo=memo,visiting=sorted(visiting)))
        memo[symbol]=value['line'];return value['line']
    backend.replace(module,'_transitive_line_to_target_symbols',transitive)
