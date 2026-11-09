"""Transport authority source and registry arguments to native rules; preserve complete DTOs."""


def configuration(module):
    signed=[(c.module,c.function) for c in module.EXPLICIT_EXECUTION_AUTHORITY_CALLSITES]
    curated=sorted(module.CURATED_EXPLICIT_AUTHORITY_CALLSITES)
    private=sorted(module.PRIVATE_SYMBOL_TEST_EXCEPTIONS)
    explicit=sorted(module.EXPLICIT_AUTHORITY_TEST_EXCEPTIONS)
    values={}
    for prefix,rows in [('signed',signed),('curated',curated),('privateException',private),('explicitException',explicit)]:
        values[prefix+'Modules']=[r[0] for r in rows]
        values[prefix+'Functions']=[r[1] for r in rows]
    values.update(privateSymbols=sorted(module.PRIVATE_AUTH_SYMBOLS),monkeypatchMarkers=list(module.AUTH_MONKEYPATCH_MARKERS))
    return values


def install(suite,module):
    def result(entry,files=(),options=None):
        rows=suite.native('execution_authority_bypass',entry,list(files),dict(configuration(module),**(options or {})))
        if any(row['code']!='AUTH_ROW' for row in rows):raise ValueError('Unexpected authority API diagnostic')
        return [module.AuthorityViolation(**{k:v for k,v in row['parameters'].items() if k!='ordinal'})
                for row in sorted(rows,key=lambda r:r['parameters']['ordinal'])]
    def scan_source(source,*,rel_path,tree=None):
        if tree is not None:raise NotImplementedError('Structured AST authority arguments remain outside this scanner checkpoint')
        return result('scan_source',[dict(path='input.py',source=source)],dict(argumentPath=rel_path))
    def registry():return result('_registry_violations')
    suite.replace(module,'scan_source',scan_source)
    suite.replace(module,'_registry_violations',registry)
