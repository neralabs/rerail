"""Transport typed AST arguments and file facts; execute extraction and reporting natively."""
import argparse,ast,sys
from pathlib import Path
from ast_transport import columns


def install(suite,module):
    def maxima():return {key:str(value) for key,value in vars(module).items() if key.startswith('MAX_')}
    def native(entry,files=(),options=None):return suite.native('grammar_allowlist_burndown',entry,list(files),dict(maxima(),**(options or {})))
    specs=native('configuration')
    for row in specs:
        p=row['parameters'];suite.replace(module,p['key'],p['maximum'])
    def allowlist_specs():
        rows=native('_allowlist_specs');return {r['parameters']['variable']:(r['parameters']['description'],r['parameters']['maximum']) for r in sorted(rows,key=lambda r:r['parameters']['ordinal'])}
    suite.replace(module,'_allowlist_specs',allowlist_specs)
    def argument(entry,node):
        data=columns(node);pending=[node];values=[]
        while pending:
            current=pending.pop();values.append(current.value if isinstance(current,ast.Constant) and isinstance(current.value,str) else '')
            pending.extend(reversed(list(ast.iter_child_nodes(current))))
        data['astConstantText']=values
        return {r['parameters']['value'] for r in native(entry,options=data) if r['code']=='GAB_VALUE'}
    suite.replace(module,'_extract_set_values',lambda node:argument('_extract_set_values',node))
    suite.replace(module,'_extract_frozenset_values',lambda node:argument('_extract_frozenset_values',node))
    def scan(entry,root,paths,raise_syntax_errors,variable_name=None):
        documents=[];original=[];relative=[]
        for value in sorted(set(paths)):
            path=value if value.is_absolute() else root/value
            if not path.is_file():continue
            try:source=path.read_text(encoding='utf-8')
            except (OSError,UnicodeDecodeError):continue
            logical='input'+str(len(documents))+path.suffix
            documents.append(dict(path=logical,source=source));original.append(str(path))
            try:relative.append(path.relative_to(root).as_posix())
            except ValueError:relative.append(path.as_posix())
        if not documents:return []
        opts=dict(documentPaths=[d['path'] for d in documents],originalPaths=original,scopePaths=relative,originalNames=[Path(p).name for p in original],raiseSyntaxErrors=raise_syntax_errors)
        if variable_name is not None:opts['variableName']=variable_name
        rows=native(entry,documents,opts)
        errors=[r['parameters']['error'] for r in rows if r['code']=='GAB_SYNTAX']
        if errors:raise SyntaxError(errors[0])
        if entry=='from_ast':return {r['parameters']['value'] for r in rows if r['code']=='GAB_VALUE'}
        values={}
        for row in rows:
            if row['code']!='GAB_FACT':continue
            p=row['parameters'];key=(row['path'],p['ordinal'],p['target_ordinal'])
            if key not in values:
                path=Path(row['path'])
                try:rel=path.relative_to(root).as_posix()
                except ValueError:rel=path.as_posix()
                values[key]=dict(path=rel,variable_name=p['variable_name'],entries=[],count=p['count'])
            values[key]['entries'].append(p['entry'])
        result=[values[key] for key in sorted(values)]
        for value in result:value['entries'].sort()
        return result
    def facts(root,files,*,raise_syntax_errors=False):return scan('facts',root,files,raise_syntax_errors)
    suite.replace(module,'_allowlist_facts_for_files',facts)
    def from_ast(file_path,variable_name):
        file_path.read_text(encoding='utf-8')
        result=scan('from_ast',file_path.resolve().parents[1],(file_path,),True,variable_name)
        return set(result)
    suite.replace(module,'_extract_frozenset_from_ast',from_ast)
    suite.replace(module,'_extract_frozenset_from_source',lambda source,variable_name,file_path=None,tree=None:set() if file_path is None else from_ast(file_path,variable_name))
    def cli_main():
        parser=argparse.ArgumentParser(description='Track grammar allowlist burndown')
        parser.add_argument('--check',action='store_true',help='CI mode: exit 1 if any allowlist exceeds its maximum')
        parser.add_argument('--verbose',action='store_true',help='Show all allowlisted entries')
        args=parser.parse_args();directory=Path(module.__file__).parent
        files=(directory/'lint_grammar_render_methods.py',directory/'lint_sql_grammar_escape_hatches.py')
        for path in files:
            if not path.is_file():raise FileNotFoundError(path)
        facts=module._allowlist_facts_for_files(directory.parent,files,raise_syntax_errors=True)
        variables=[];paths=[];entries=[]
        for fact in facts:
            for value in fact['entries']:
                variables.append(fact['variable_name']);paths.append(fact['path']);entries.append(value)
        rows=native('report',options=dict(factVariables=variables,factPaths=paths,factEntries=entries,verbose=args.verbose))
        output=sorted((r['parameters'] for r in rows if r['code']=='GAB_OUTPUT'),key=lambda p:(p['section'],p['part'],p['order']))
        sys.stdout.write(''.join(p['text'] for p in output))
        return next(r['parameters']['status'] for r in rows if r['code']=='GAB_EXIT')
    suite.replace(module,'cli_main',cli_main)

    def plugin(root):
        def report(view):
            data={key:[] for key in ['reportNames','reportPaths','reportFileOrders','reportOrdinals','reportTargets','reportCounts']}
            for file_order,bundle in enumerate(view.files.values()):
                raw=bundle.facts.get(module.GRAMMAR_ALLOWLIST_BURNDOWN_EXTRACTOR.extractor_id,[])
                if not isinstance(raw,list):continue
                for ordinal,fact in enumerate(raw):
                    if not isinstance(fact,dict):continue
                    values=[str(fact.get('variable_name','')),str(fact.get('path','scripts')),str(file_order),str(ordinal),'0',str(fact.get('count',0))]
                    for key,value in zip(data,values):data[key].append(value)
            rows=native('reporter_facts',options=data)
            return [module.StructuredViolation(linter_name='grammar_allowlist_burndown',code=r['code'],path=root/r['path'],line=r['line'],message=r['message'],severity='hard-fail') for r in sorted(rows,key=lambda r:r['parameters']['spec_ordinal']) if r['code']=='allowlist_grew']
        return module.LinterPlugin(name='grammar_allowlist_burndown',enforcement='hard-fail',description='SQL grammar linter allowlists must not grow',file_scope=module.FileScopeSpec(directories=(root/'scripts',),include_tests=False),required_facts=(module.GRAMMAR_ALLOWLIST_BURNDOWN_EXTRACTOR,),reporter=report)
    suite.replace(module,'grammar_allowlist_burndown_plugin',plugin)


def cli_probe(module,values):
    previous_file,previous_args=module.__file__,sys.argv
    previous_maxima={key:getattr(module,key) for key in values.get('maxima',{})}
    for key,value in values.get('maxima',{}).items():setattr(module,key,value)
    try:
        module.__file__=str(values['root']/'scripts/lint_grammar_allowlist_burndown.py')
        sys.argv=['lint_grammar_allowlist_burndown.py',*values['args']]
        return module.cli_main()
    finally:
        module.__file__=previous_file;sys.argv=previous_args
        for key,value in previous_maxima.items():setattr(module,key,value)


def reporter_probe(module,values):
    from types import SimpleNamespace
    root=values['root'];bundles={}
    for ordinal,facts in enumerate(values['files']):
        bundles[root/str(ordinal)]=SimpleNamespace(facts={module.GRAMMAR_ALLOWLIST_BURNDOWN_EXTRACTOR.extractor_id:facts})
    return module.grammar_allowlist_burndown_plugin(root).reporter(SimpleNamespace(files=bundles))
