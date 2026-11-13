"""Transport plugin inputs; native rules own defaults, filtering, coercion and ordering."""
import json
from pathlib import Path
from value_columns import columns

CHECK='plan_scope_side_effects'


def install(backend,module):
    rows=backend.native(CHECK,'_pse_plugin_metadata',[])
    if len(rows)!=1 or rows[0]['code']!='PSE_PLUGIN_META':raise ValueError('One native plugin descriptor required')
    meta=rows[0]['parameters']
    fields=meta['value_fields'].split(',');severity=meta['severity_field'];missing=object()
    def shape(value):return 'dict'if isinstance(value,dict)else 'list'if isinstance(value,list)else type(value).__name__
    def private(root,scope):
        def report(view):
            # Capture the selector's declared columns first. Do not traverse
            # unused descendants or rows before the native demand selects them.
            family=module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id
            bundles=list(view.files.values());headers=[];sources=[]
            for b in bundles:
                value=b.facts.get(family,[]);sources.append(value);items=[]
                if isinstance(value,list):
                    for row in value:
                        header=dict(kind=shape(row))
                        if isinstance(row,dict):
                            observed=row.get(severity,missing)
                            if observed is not missing:header['severity']=observed
                        items.append(header)
                headers.append(dict(container=shape(value),rows=items))
            selected=backend.native(CHECK,'_pse_plugin_select',[],columns(headers))
            if any(r['code']!='PSE_PLUGIN_ROW'for r in selected):raise ValueError('Unexpected native row demand')
            if not selected:return []
            demands={}
            for row in selected:
                p=row['parameters'];demands.setdefault(p['bundle'],[]).append(p['ordinal'])
            values=[]
            for b,ordinals in sorted(demands.items()):
                projected=[{key:sources[b][i][key]for key in fields if key in sources[b][i]}for i in sorted(ordinals)]
                values.append(dict(path=bundles[b].path.as_posix(),facts={family:projected}))
            rows=backend.native(CHECK,'_pse_plugin_report',[],dict(columns(values),psePluginRoot=root.as_posix()))
            if any(r['code']!='PSE_PLUGIN_FINDING'for r in rows):raise ValueError('Unexpected native plugin report')
            values=sorted((r['parameters']for r in rows),key=lambda p:(p['path_key'],p['line_sign'],p['line_length'],p['line_digits'],p['source_code'],p['bundle'],p['ordinal']))
            return [module.StructuredViolation(meta['name'],p['source_code'],Path(p['path']),json.loads(p['line_json']),p['source_message'],meta['enforcement'])for p in values]
        return module.LinterPlugin(meta['name'],meta['enforcement'],meta['description'],scope,(module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR,),report)
    def public(root):
        rows=backend.native(CHECK,'_pse_plugin_roots',[],root=root)
        if any(r['code']!='PSE_PLUGIN_ROOT'for r in rows):raise ValueError('Unexpected native plugin roots')
        paths=tuple(root/r['parameters']['path']for r in sorted(rows,key=lambda r:r['parameters']['ordinal']))
        return private(root,module.FileScopeSpec(directories=paths,include_tests=False))
    backend.replace(module,'_plan_scope_side_effects_plugin',private)
    backend.replace(module,'plan_scope_side_effects_plugin',public)
