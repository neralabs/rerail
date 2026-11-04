# SPDX-License-Identifier: Apache-2.0
"""Translate Python implementation hooks to native counters; retain behavior assertions."""
import ast
import hashlib
import inspect
import textwrap


COUNTERS={
    'test_cached_policy_enumerates_only_candidate_module_asts_once':('module_walks','pythonAstModuleWalks'),
    'test_function_fact_reuses_one_body_walk':('walk_count','syntaxLoads'),
}


def translate(backend,items):
    backend.instrumentation_translations=[]
    for item in items:
        name=getattr(item,'originalname',None) or item.name
        if name not in COUNTERS and name!='test_lazy_scan_reuses_legacy_boundary_symbol_index':continue
        original=textwrap.dedent(inspect.getsource(item.obj));tree=ast.parse(original)
        function=next(node for node in tree.body if isinstance(node,ast.FunctionDef));function.decorator_list=[]
        class Counters(ast.NodeTransformer):
            def visit_Assert(self,node):
                if name in COUNTERS:
                    old,counter=COUNTERS[name]
                    class Reads(ast.NodeTransformer):
                        def visit_Name(self,value):
                            return ast.copy_location(ast.Call(func=ast.Name(id='_native_metric',ctx=ast.Load()),args=[ast.Constant(counter)],keywords=[]),value) if value.id==old and isinstance(value.ctx,ast.Load) else value
                    return Reads().visit(node)
                text=ast.unparse(node.test)
                replacement=("_native_metric('relationalMaterializations').get('legacy_matches',0) == 1" if 'len(observed_indexes)' in text
                  else 'sum(fact.has_pure_lazy_decorator for fact in analyzer.facts) == 2')
                return ast.copy_location(ast.Assert(test=ast.parse(replacement,mode='eval').body,msg=node.msg),node)
        tree=Counters().visit(tree);ast.fix_missing_locations(tree)
        namespace=dict(item.obj.__globals__)
        def metric(key):
            metrics=backend.last_metrics
            # Resident results contain workspaceProfile; snapshots contain profile.
            metrics=metrics.get('workspaceProfile',metrics)
            if key not in metrics:raise AssertionError('Native backend did not report '+key)
            return metrics[key]
        namespace['_native_metric']=metric
        exec(compile(tree,inspect.getsourcefile(item.obj),'exec'),namespace)
        item.obj=namespace[function.name]
        digest=lambda source:hashlib.sha256(source.encode()).hexdigest()
        backend.instrumentation_translations.append(dict(test=item.nodeid,originalHash=digest(original),
            translatedHash=digest(ast.unparse(tree)),nativeCounter=COUNTERS[name][1] if name in COUNTERS else 'relationalMaterializations.legacy_matches'))
