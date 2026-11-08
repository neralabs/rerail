"""Transport graph arguments as columns; run dead-code decisions in native rules."""
import json
import inspect
from pathlib import Path
import types


def graph_algorithm_codes(module):
    """Guard imported graph algorithms too; graph storage construction is host protocol."""
    result={}
    def register(code,name):
        result[code]=('internal_dead_code.symbol_graph',name,False)
        for value in code.co_consts:
            if isinstance(value,types.CodeType):register(value,name)
    for name,value in vars(module.SymbolGraph).items():
        if name=='__init__':continue
        if isinstance(value,(staticmethod,classmethod)):value=value.__func__
        if inspect.isfunction(value):register(value.__code__,value.__qualname__)
    return result


def graph_columns(graph):
    data={name:[] for name in ['symbolIds','symbolFqNames','symbolNames','symbolModules','symbolKinds','symbolPaths','symbolLines','symbolOwners','symbolHasOwner','symbolAbstract','flagSymbols','flagNames','decoratorSymbols','decoratorTexts','baseNameSymbols','baseNameTexts','baseClasses','baseTargets','baseOrdinals','subclassBases','subclassTargets','methodClasses','methodNames','methodSymbols','fqNames','fqSymbols','modules','importModules','importLocals','importTargets','moduleImportModules','moduleImportTargets','definedModules','definedNames','definedSymbols','refModules','refNames','annotationSymbols','annotationNames','annotationFiles','annotationFileSymbols','callSources','callTargets','useSources','useTargets','providedPublicClasses','providedPublicSymbols','providedDunderClasses','providedDunderSymbols']}
    documents=[]
    for sid,symbol in graph.symbols.items():
        raw=vars(symbol)
        data['symbolFqNames'].append(raw.get('fqname',''))
        for column,key,default in [('symbolIds',None,sid),('symbolNames','name',''),('symbolModules','module',''),('symbolKinds','kind',''),('symbolPaths','path',''),('symbolLines','line',0),('symbolOwners','class_name',''),('symbolAbstract','is_abstract',False)]:
            value=raw.get(key,default) if key else default
            if column=='symbolAbstract':value=int(value)
            data[column].append('' if value is None else str(value))
        data['symbolHasOwner'].append(str(int(raw.get('class_name') is not None)))
        if 'is_class' in raw:
            data['flagSymbols'].append(sid);data['flagNames'].append('explicit_is_class')
            if raw['is_class']:data['flagSymbols'].append(sid);data['flagNames'].append('is_class')
        for key,left,right in [('decorators','decoratorSymbols','decoratorTexts'),('bases','baseNameSymbols','baseNameTexts'),('calls','callSources','callTargets'),('class_uses','useSources','useTargets')]:
            for value in raw.get(key,[]):data[left].append(sid);data[right].append(value)
        cached=raw.get('_annotation_names_cache',raw.get('annotation_names'))
        if cached is not None:
            for name in cached:data['annotationSymbols'].append(sid);data['annotationNames'].append(name)
        else:
            for annotation in [raw.get('return_annotation'),*raw.get('param_annotations',[])]:
                if annotation:
                    path=f'annotations/input_{len(documents)}.py'
                    documents.append(dict(path=path,source='__rerail_annotation = (\n'+annotation+'\n)\n'))
                    data['annotationFiles'].append(path);data['annotationFileSymbols'].append(sid)
    for fq,sid in graph.symbol_by_fqname.items():data['fqNames'].append(fq);data['fqSymbols'].append(sid)
    for owner,items in getattr(graph,'class_bases',{}).items():
        for i,target in enumerate(items):data['baseClasses'].append(owner);data['baseTargets'].append(target);data['baseOrdinals'].append(str(i))
    for owner,items in getattr(graph,'class_subclasses',{}).items():
        for target in items:data['subclassBases'].append(owner);data['subclassTargets'].append(target)
    for owner,items in getattr(graph,'class_methods',{}).items():
        for name,sid in items.items():data['methodClasses'].append(owner);data['methodNames'].append(name);data['methodSymbols'].append(sid)
    for module,info in getattr(graph,'modules',{}).items():
        data['modules'].append(module)
        for local,fq in info.imported_symbols.items():data['importModules'].append(module);data['importLocals'].append(local);data['importTargets'].append(fq)
        for target in info.imported_modules.values():data['moduleImportModules'].append(module);data['moduleImportTargets'].append(target)
        for name,ids in info.defined_names.items():
            for sid in ids:data['definedModules'].append(module);data['definedNames'].append(name);data['definedSymbols'].append(sid)
        for name in info.module_level_names:data['refModules'].append(module);data['refNames'].append(name)
    # The original tests supply an opaque graph double whose class accessor rows
    # are input facts. Never call the production Python graph algorithms here.
    if not hasattr(graph,'class_methods'):
        for sid,symbol in graph.symbols.items():
            if vars(symbol).get('is_class'):
                for target in graph.public_methods_of_class(sid):data['providedPublicClasses'].append(sid);data['providedPublicSymbols'].append(target)
                for target in graph._class_dunder_methods(sid):data['providedDunderClasses'].append(sid);data['providedDunderSymbols'].append(target)
    data['exportedRoots']=list(graph.exported_root_ids)
    data['suppliedGraph']=True
    return data,documents


def result_columns(results):
    data=dict(resultCategories=list(results),resultRowCategories=[],resultRowSymbols=[],resultRowOrdinals=[])
    for category,ids in results.items():
        for i,sid in enumerate(ids):data['resultRowCategories'].append(category);data['resultRowSymbols'].append(sid);data['resultRowOrdinals'].append(str(i))
    return data


def install(suite,module):
    def native(entry,graph=None,options=None):
        data,documents=graph_columns(graph) if graph is not None else ({},[])
        data.update(options or {})
        return suite.native('internal_dead_code',entry,documents,data)
    def values(rows):return {r['parameters']['value'] for r in rows if r['code']=='IDC_VALUE'}
    suite.replace(module,'_collect_literal_imported_fqnames',lambda source:values(native('_collect_literal_imported_fqnames',options=dict(source=source))))
    def dynamic_roots(graph,imported_fqnames):
        rows=native('_resolve_dynamic_import_roots',graph,dict(importedFqNames=list(imported_fqnames)))
        missing=[row for row in rows if row['code']=='IDC_GRAPH_METHOD_MISSING']
        if missing:raise KeyError(missing[0]['parameters']['sid'])
        return values(rows)
    suite.replace(module,'_resolve_dynamic_import_roots',dynamic_roots)
    def filtered(results,graph,excluded_paths):
        rows=native('_filter_results_by_excluded_paths',graph,dict(**result_columns(results),excludedPaths=list(excluded_paths)))
        if any(r['code']=='IDC_KEEP_RESULTS' for r in rows):return results
        return grouped(rows)
    def grouped(rows):
        headers=sorted([r['parameters'] for r in rows if r['code']=='IDC_GROUP'],key=lambda p:p['ordinal'])
        items=[r['parameters'] for r in rows if r['code']=='IDC_ITEM']
        return {p['category']:[v['sid'] for v in sorted(items,key=lambda v:v['ordinal']) if v['category']==p['category']] for p in headers}
    suite.replace(module,'_filter_results_by_excluded_paths',filtered)
    def targets(results,graph,*,categories,paths,extra_root_ids=None):
        rows=native('_collect_execute_targets',graph,dict(**result_columns(results),categories=categories or [],paths=paths or [],extraRoots=list(extra_root_ids or [])))
        items=sorted([r['parameters'] for r in rows if r['code']=='IDC_TARGET'],key=lambda p:p['ordinal'])
        output={}
        for row in items:output.setdefault(row['path'],[]).append(row['sid'])
        return output
    suite.replace(module,'_collect_execute_targets',targets)
    def analyze(graph,*,extra_root_ids=None,excluded_paths=()):
        return grouped(native('analyze_dead_code',graph,dict(extraRoots=list(extra_root_ids or []),excludedPaths=list(excluded_paths))))
    suite.replace(module,'analyze_dead_code',analyze)
