"""Transport plugin inputs; native rules own defaults, filtering, coercion and ordering."""
import json
from pathlib import Path
from value_columns import columns

CHECK='plan_scope_side_effects'


def install(backend,module):
    rows=backend.native(CHECK,'_pse_plugin_metadata',[])
    if len(rows)!=1 or rows[0]['code']!='PSE_PLUGIN_META':raise ValueError('One native plugin descriptor required')
    meta=rows[0]['parameters']
    def private(root,scope):
        def report(view):
            # Only the fact families in this plugin's declared demand cross the
            # boundary. Preserve values and insertion positions without coercion.
            wanted={module.PLAN_SCOPE_SIDE_EFFECTS_EXTRACTOR.extractor_id}
            values=[dict(path=b.path.as_posix(),facts={k:v for k,v in b.facts.items()if k in wanted})for b in view.files.values()]
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
