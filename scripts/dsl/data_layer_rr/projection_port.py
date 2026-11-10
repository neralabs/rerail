"""Construct passive DTOs from native, typed and ordered model projections."""
import json
from pathlib import Path
import types
import typing

from migration import ROOT,compare
from replay import execute_workspace
from model_transport import encode_models
from surface_reporter_port import report_rows


class ProjectionTypeError(AssertionError):
    def __init__(self,expected,actual):
        self.expected,self.actual=expected,actual
        super().__init__(f'Native projection supplied {actual}, expected {expected}')


def construct(cls,data):
    """Use storage constructors only. Primitive coercion belongs to native rules."""
    assert isinstance(data,dict)
    def value(annotation,raw):
        if typing.get_origin(annotation) in {typing.Union,types.UnionType}:
            options=typing.get_args(annotation)
            if raw is None:
                assert type(None) in options;return None
            annotation=next(a for a in options if a is not type(None))
        annotation=getattr(annotation,'__supertype__',annotation)
        if typing.get_origin(annotation) is tuple:
            assert isinstance(raw,list)
            return tuple(value(typing.get_args(annotation)[0],v) for v in raw)
        if isinstance(annotation,type) and hasattr(annotation,'model_fields'):return construct(annotation,raw)
        if type(raw) is not annotation:raise ProjectionTypeError(annotation.__name__,type(raw).__name__)
        return raw
    fields={}
    for key,field in cls.model_fields.items():
        if key in data:fields[key]=value(field.annotation,data[key])
        else:assert not field.is_required(),key
    return cls.model_construct(_fields_set=set(fields),**fields)


def project_rows(rows,shared):
    # This raises the native error DTO. It performs no model validation.
    report_rows(rows,lambda **fields:None)
    ordered=sorted((r['parameters'] for r in rows if r['code']=='SF_MODEL'),key=lambda p:(p['path_order'],p['bundle']))
    return shared.ProjectServerPlanFacts.model_construct(files=tuple(
        construct(shared.ServerPlanFileFacts,json.loads(p['normalized_json'])) for p in ordered))


def native_case(engine,case,shared_module=None,policy=None):
    from projection_contract import describe,error_value
    if shared_module is None:import model_dtos as shared_module
    policy=policy or ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-grammar-surface-project.rr'
    # ProjectFactView is a mapping keyed by Path. Preserve its ordinary storage
    # semantics: an equal key replaces the value and retains its first position.
    bundles={Path(b['path']):b for b in case['bundles']}
    execution=execute_workspace(engine,policy,[dict(path='reporter-facts.json',
        source=json.dumps(dict(bundles=encode_models(list(bundles.values()))),ensure_ascii=False))],dict(entryPoint='_project_facts'))
    from pydantic import ValidationError
    try:project=project_rows(execution['diagnostics'],shared_module)
    except ValidationError as error:return error_value(error),[execution]
    except ProjectionTypeError as error:
        error.executions=[execution];raise
    bundles=[dict(path=str(i),value=file.model_dump(mode='json')) for i,file in enumerate(project.files)]
    message_execution=execute_workspace(engine,policy,[dict(path='reporter-facts.json',
        source=json.dumps(dict(bundles=encode_models(bundles)),ensure_ascii=False))],dict(entryPoint='_project_messages',
        reportRoot=str(Path(case.get('messageRoot','root')))))
    messages=message_rows(message_execution['diagnostics'])
    return dict(model=describe(project),messages=messages),[execution,message_execution]


def message_rows(rows):
    report_rows(rows,lambda **fields:None)
    return [r['parameters']['value'] for r in sorted((r for r in rows if r['code']=='SPG_MESSAGE'),
        key=lambda r:(r['parameters']['bundle'],r['parameters']['ordinal']))]
