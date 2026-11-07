"""Transport async collector inputs and passive DTOs; all decisions execute in .rr."""
from pathlib import Path
import sys
from ast_transport import syntax_columns


def scan(suite, entry, module, args):
    path = Path(args.get('filepath', 'input.py'))
    if entry == 'should_exclude':
        rows = suite.native('async_patterns', entry, [], dict(originalPath=str(path)))
        if len(rows)>1 or any(row['code']!='AP_EXCLUDED' for row in rows):
            raise AssertionError('Invalid native async exclusion predicate')
        return bool(rows)
    options = dict(originalPath=str(path), analysisPath=path.name)
    error = None
    if entry == 'analyze_source':
        source = args['source']
        tree = args.get('tree')
        options.update(syntax_columns(tree) if tree is not None else {})
        transport = options.pop('receiverTransport', 'pass\n')
        options.update(providedTree=str(tree is not None).lower(), providedSource=source, sourceLines=source.splitlines())
        files = [dict(path='impl/common/_receiver_input.py', source=transport)]
    else:
        # The source API excludes paths before opening them. The rule owns that predicate.
        if scan(suite, 'should_exclude', module, args):
            return []
        try:
            source = path.read_text(encoding='utf-8')
        except (OSError, UnicodeDecodeError) as caught:
            error, source = caught, ''
        options.update(ioError=str(isinstance(error, OSError)).lower(), decodeError=str(isinstance(error, UnicodeDecodeError)).lower())
        # Path components remain input data: exclusion, scope, and fallback decisions are native.
        logical = path.as_posix().lstrip('/') if path.is_absolute() else path.as_posix()
        files = [dict(path=logical, source=source)]
    rows = suite.native('async_patterns', entry, files, options)
    for row in rows:
        if row['code']=='AP_AST_REQUIRED':
            if len(rows)!=1:raise AssertionError('Mixed missing-AST and detection results')
            raise ValueError('AST tree is required; parsing is owned by the fact framework')
        if row['code']=='AP_READ_ERROR':
            if len(rows)!=1 or not isinstance(error,OSError):raise AssertionError('Invalid native read error')
            raise error
        if row['code']=='AP_PARSE_WARNING':
            if len(rows)!=1 or not isinstance(error,UnicodeDecodeError):raise AssertionError('Invalid native decoding warning')
            print(f'Warning: Could not parse {path}: {error}',file=sys.stderr)
            return []
    if any(row['code'] not in {'ASYNC001','ASYNC002','ASYNC004','ASYNC005','ASYNC006','ASYNC007','ASYNC008'} for row in rows):
        raise AssertionError('Invalid async pattern diagnostic')
    return [module.Violation(file=path,line=row['line'],code=row['code'],message=row['message'],
        context=row['parameters']['context'],is_fallback=bool(row['parameters']['fallback']))
        for row in sorted(rows,key=lambda row:(row['parameters']['ordinal'],row['parameters']['rank']))]
