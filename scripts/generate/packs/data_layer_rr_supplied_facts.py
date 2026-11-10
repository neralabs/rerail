"""Configure native reporters to match the pinned plugin's public fact contract."""
import json
import re
import importlib
import typing
import types
from data_layer_rr_text import search


def supplied_facts(original, check):
    if check == 'serverplan_grammar_surface': return surface_schema()
    configs = {
        'removed_compat_surfaces': dict(path_field='path', join_root='true', message='removed', code='RCS001', order='input'),
        'versioned_lifecycle_legacy_surfaces': dict(path_field='file_path', join_root='true', message='field', code='VLL001', order='message'),
        'no_api_root_imports': dict(path_field='file_path', message='field', code='NARI001', order='message'),
        'backend_type_strings': dict(path_field='file', message='field', code_field='code', order='code'),
        'hasattr_async': dict(path_field='bundle', message='async', code_field='pattern_type', strict_line='true', order='bundle'),
        'no_source_inspection': dict(path_field='file_path', message='inspection', code='NSI001', order='message'),
        'serverplan_name_inference': dict(path_field='bundle', message='field', code_field='code', strict_line='true', order='bundle'),
        'no_blanket_grammar_bypass': dict(path_field='file_path', join_root='true', message='field', code='NGB001', order='message'),
        'permission_error': dict(path_field='path', message='field', code='builtin_permission_error', order='line'),
    }
    defaults = {
        'removed_compat_surfaces': dict(name='removed compatibility surface'),
        'versioned_lifecycle_legacy_surfaces': dict(message='Versioned lifecycle legacy surface violation'),
        'no_api_root_imports': dict(message='deprecated root api import'),
        'backend_type_strings': dict(code='BT000', message='Backend type string comparison'),
        'hasattr_async': {}, 'no_source_inspection': dict(pattern_name='source inspection'),
        'serverplan_name_inference': {},
        'no_blanket_grammar_bypass': dict(message='ServerPlan bypass mechanism found'),
        'permission_error': dict(message='Raises built-in PermissionError'),
    }
    strict = {'hasattr_async': ['pattern_type', 'method_name'], 'serverplan_name_inference': ['code', 'message']}
    text = '\n#include "data-layer/supplied-facts.rr"\n'
    text += ''.join(f'sf_config({json.dumps(k)},{json.dumps(v)}).\n' for k,v in configs[check].items())
    text += ''.join(f'sf_default({json.dumps(k)},{json.dumps(v)}).\n' for k,v in defaults[check].items())
    text += ''.join(f'sf_strict({json.dumps(k)}).\n' for k in strict.get(check, []))
    if check == 'hasattr_async':
        for pattern in original.EXCLUDE_PATTERNS:
            text += 'sf_excluded(b) :- sf_bundle(b,p), workspace_root(root), match('+json.dumps(search(re.compile(pattern)))+',cat(root,"/",p)).\n'
    return text


def surface_schema():
    shared=importlib.import_module('scripts.lint_lib.serverplan_facts')
    text='\n#include "data-layer/surface-supplied-facts.rr"\n'
    for name in ['ServerPlanFileFacts','ServerPlanClassFacts','ServerPlanFunctionFacts',
                 'ServerPlanCompileMethodFacts','GrammarSurfaceHit']:
        for i,(key,field) in enumerate(getattr(shared,name).model_fields.items()):
            annotation=field.annotation; nullable=0
            if typing.get_origin(annotation) in {typing.Union,types.UnionType}:
                options=typing.get_args(annotation); assert type(None) in options and len(options)==2
                annotation=next(a for a in options if a is not type(None)); nullable=1
            annotation=getattr(annotation,'__supertype__',annotation); item=''
            if typing.get_origin(annotation) is tuple:
                kind='tuple'; child=getattr(typing.get_args(annotation)[0],'__supertype__',typing.get_args(annotation)[0])
                item='string' if child is str else child.__name__
            else: kind={str:'string',int:'int',bool:'bool'}[annotation]
            text+='sv_schema('+','.join([json.dumps(name),json.dumps(key),str(i),json.dumps(kind),
                str(int(field.is_required())),str(nullable),json.dumps(item)])+').\n'
    return text
