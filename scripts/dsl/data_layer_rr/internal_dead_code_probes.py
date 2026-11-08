"""Construct supplied graph arguments; keep source algorithms out of native adapters."""
import importlib
from pathlib import Path


def graph_argument(module, root, values):
    model=importlib.import_module(module.SymbolGraph.__module__)
    graph=module.SymbolGraph(root)
    for raw in values.get('symbols',[]):
        data=dict(raw)
        sid=data.pop('sid')
        module_name,qual=sid.split('::',1)
        data.setdefault('symbol_id',sid);data.setdefault('fqname',module_name+'.'+qual)
        data.setdefault('module',module_name);data.setdefault('qualname',qual);data.setdefault('name',qual.rsplit('.',1)[-1])
        data.setdefault('kind','function');data.setdefault('path',module_name.replace('.','/')+'.py');data.setdefault('line',1)
        for key in ['calls','class_uses','execution_hits']:data[key]=set(data.get(key,[]))
        for key in ['decorators','class_decorators','param_names','param_annotations','bases','class_attr_annotations']:data[key]=tuple(data.get(key,[]))
        symbol=model.SymbolInfo(**data);graph.symbols[sid]=symbol;graph.symbol_by_fqname[symbol.fqname]=sid
        info=graph.modules.setdefault(module_name,model.ModuleInfo(module_name,Path(symbol.path)))
        if symbol.class_name is None and '.' not in qual:info.defined_names[symbol.name].append(sid)
        if symbol.is_class:graph.class_ids_by_name[symbol.name].append(sid)
        if symbol.kind=='method' and symbol.class_name is not None:graph.class_methods[module_name+'::'+symbol.class_name][symbol.name]=sid
    for name,raw in values.get('modules',{}).items():
        info=graph.modules.setdefault(name,model.ModuleInfo(name,Path(raw.get('path',name.replace('.','/')+'.py'))))
        for field in ['imported_symbols','imported_modules','defined_names']:
            if field in raw:setattr(info,field,dict(raw[field]))
        if 'module_level_names' in raw:info.module_level_names=set(raw['module_level_names'])
    for owner,bases in values.get('class_bases',{}).items():
        graph.class_bases[owner]=list(bases)
        for base in bases:graph.class_subclasses[base].add(owner)
    if 'class_methods' in values:
        graph.class_methods={owner:dict(methods) for owner,methods in values['class_methods'].items()}
    graph.exported_root_ids=set(values.get('roots',[]))
    return graph


def probe(module,root,values):
    graph=graph_argument(module,root,values['graph'])
    result=module.analyze_dead_code(graph,extra_root_ids=set(values.get('extra_roots',[])),excluded_paths=tuple(values.get('excluded_paths',[])))
    return dict(results=result,exported_roots=graph.exported_root_ids)


def dynamic_roots_probe(module,root,values):
    graph=graph_argument(module,root,values['graph'])
    return module._resolve_dynamic_import_roots(graph,set(values['imported_fqnames']))
