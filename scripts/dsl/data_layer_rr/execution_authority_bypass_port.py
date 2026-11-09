"""Transport authority source and registry arguments to native rules; preserve complete DTOs."""


def configuration(module):
    signed=[(c.module,c.function) for c in module.EXPLICIT_EXECUTION_AUTHORITY_CALLSITES]
    curated=sorted(module.CURATED_EXPLICIT_AUTHORITY_CALLSITES)
    private=sorted(module.PRIVATE_SYMBOL_TEST_EXCEPTIONS)
    explicit=sorted(module.EXPLICIT_AUTHORITY_TEST_EXCEPTIONS)
    values=dict(signedCount=str(len(signed)),env=['auth.api=true','auth.config=arguments'])
    for prefix,rows in [('signed',signed),('curated',curated),('privateException',private),('explicitException',explicit)]:
        values[prefix+'Modules']=[r[0] for r in rows]
        values[prefix+'Functions']=[r[1] for r in rows]
    values.update(privateSymbols=sorted(module.PRIVATE_AUTH_SYMBOLS),monkeypatchMarkers=list(module.AUTH_MONKEYPATCH_MARKERS))
    return values


def install(suite,module):
    def result(entry,files=(),options=None):
        rows=suite.native('execution_authority_bypass',entry,list(files),dict(configuration(module),**(options or {})))
        errors=[row for row in rows if row['code']=='AUTH_PATH_ERROR']
        if errors:
            if len(rows)!=1:raise ValueError('A path error must be the only native result')
            raise ValueError(errors[0]['parameters']['detail'])
        if any(row['code']!='AUTH_ROW' for row in rows):raise ValueError('Unexpected authority API diagnostic')
        return [module.AuthorityViolation(**{k:v for k,v in row['parameters'].items() if k!='ordinal'})
                for row in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
    def scan_source(source,*,rel_path,tree=None):
        return result('scan_source',[dict(path='input.py',source=source)],dict(argumentPath=rel_path,**(argument_columns(tree) if tree is not None else {})))
    def registry():return result('_registry_violations')
    suite.replace(module,'scan_source',scan_source)
    suite.replace(module,'_registry_violations',registry)
    def module_for_path(path):
        rows=suite.native('execution_authority_bypass','_module_for_path',[],dict(configuration(module),argumentPath=str(path)))
        if len(rows)!=1:raise ValueError('One native module-path result required')
        if rows[0]['code']=='AUTH_PATH_ERROR':raise ValueError(rows[0]['parameters']['detail'])
        if rows[0]['code']!='AUTH_MODULE':raise ValueError('Unexpected native module-path result')
        return rows[0]['parameters']['value']
    suite.replace(module,'_module_for_path',module_for_path)

    from pathlib import Path
    import argparse
    import sys
    default_root=module.REPO_ROOT
    def repository(entry,root,extra=None):
        options=dict(configuration(module),env=['auth.api=true','auth.root='+str(Path(root).absolute()),'auth.entry='+entry,'auth.config=arguments'],**(extra or {}))
        return suite.native('execution_authority_bypass',entry,[],options)
    def python_files(root):
        rows=repository('_paths',root)
        if any(row['code']!='AUTH_FILE' for row in rows):raise ValueError('Unexpected native file selection')
        return [Path(root)/row['parameters']['path'] for row in sorted(rows,key=lambda row:row['parameters']['path_order'])]
    def scan_repo(root=default_root):
        rows=repository('scan_repo',root)
        if any(row['code']!='AUTH_BATCH_ROW' for row in rows):raise ValueError('Unexpected native repository result')
        return [module.AuthorityViolation(**{k:row['parameters'][k] for k in ['path','line','code','message']}) for row in sorted(rows,key=lambda row:tuple(row['parameters'][k] for k in ['phase','path_order','ordinal']))]
    def cli_main(argv=None):
        parser=argparse.ArgumentParser(description=module.__doc__)
        parser.add_argument('--check',action='store_true',help='Exit non-zero when violations exist.')
        args=parser.parse_args(argv)
        values=module.scan_repo()
        rows=suite.native('execution_authority_bypass','_cli_format',[],dict(configuration(module),checkMode='1' if args.check else '0',reportPaths=[v.path for v in values],reportLines=[str(v.line) for v in values],reportCodes=[v.code for v in values],reportMessages=[v.message for v in values]))
        if any(row['code'] not in {'AUTH_OUTPUT','AUTH_EXIT'} for row in rows):raise ValueError('Unexpected native CLI result')
        exits=[row['parameters']['status'] for row in rows if row['code']=='AUTH_EXIT']
        if len(exits)!=1:raise ValueError('One native CLI exit result required')
        for row in sorted((row for row in rows if row['code']=='AUTH_OUTPUT'),key=lambda row:tuple(row['parameters'][k] for k in ['phase','path_order','ordinal'])):sys.stdout.write(row['parameters']['text'])
        return exits[0]
    suite.replace(module,'_iter_python_files',python_files)
    suite.replace(module,'scan_repo',scan_repo)
    suite.replace(module,'cli_main',cli_main)



def argument_columns(tree):
    import ast
    from ast_transport import columns
    result=columns(tree)
    result['astStrings']=[]
    pending=[tree]
    while pending:
        node=pending.pop()
        result['astStrings'].append(node.value if isinstance(node,ast.Constant) and isinstance(node.value,str) else '')
        pending.extend(reversed(list(ast.iter_child_nodes(node))))
    return result
