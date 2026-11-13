"""Apply native conversion decisions and project passive caller-owned DTO fields."""
import builtins
import json
import sys
from value_columns import columns
CHECK='plan_scope_side_effects'


def install(backend,module):
    rows=backend.native(CHECK,'_pse_conversion_configuration',[])
    if len(rows)!=1 or rows[0]['code']!='PSE_CONVERSION_CONFIG':raise ValueError('One native conversion descriptor required')
    meta=rows[0]['parameters'];fields=meta['fields'].split(',')
    def to_fact(violation):return {key:getattr(violation,key)for key in fields}
    def from_structured(root,finding):
        rows=backend.native(CHECK,'_pse_from_structured',[],dict(columns(finding.severity),pseRoot=root.as_posix(),psePath=finding.path.as_posix()))
        if len(rows)!=1 or rows[0]['code']!='PSE_STRUCTURED_CONVERSION':raise ValueError('One native structured conversion required')
        p=rows[0]['parameters']
        return module.Violation(code=finding.code,file_path=p['file_path'],line_number=finding.line,message=finding.message,severity=p['severity'])
    def extract(root,path,source,tree=None):raise getattr(builtins,meta['extract_error_type'])(meta['extract_error'])
    def from_fact(raw):
        rows=backend.native(CHECK,'_pse_from_fact',[],dict(columns(raw),pseIntMaxDigits=str(sys.get_int_max_str_digits())))
        if len(rows)!=1:raise ValueError('One native fact conversion required')
        row=rows[0];p=row['parameters']
        if row['code']=='PSE_CONVERSION_ERROR':raise getattr(builtins,p['error_type'])(p['detail'])
        if row['code']!='PSE_FACT_CONVERSION':raise ValueError('Unexpected native fact conversion')
        if not p['present']:return None
        return module.Violation(code=p['source_code'],file_path=p['file_path'],line_number=json.loads(p['line_json']),message=p['source_message'],severity=p['severity'])
    backend.replace(module,'_violation_to_fact',to_fact)
    backend.replace(module,'_violation_from_structured',from_structured)
    backend.replace(module,'_extract_plan_scope_side_effect_facts',extract)
    backend.replace(module,'_violation_from_fact',from_fact)
