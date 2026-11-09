"""Transport file arguments to the native direct-execution policy; rebuild passive DTOs."""
from pathlib import Path


def install(suite, module):
    def rows(entry, files=(), options=None):
        options=dict(options or {})
        options['env']=['ide.api=true',*options.get('env',[])]
        return suite.native('impl_direct_execution', entry, list(files), options)

    settings=rows('configuration')
    if not settings or any(row['code']!='IDE_SETTING' for row in settings):
        raise ValueError('Native direct-execution configuration is missing')
    def setting(name):
        return [r['parameters']['value'] for r in sorted(settings,key=lambda r:r['parameters']['ordinal']) if r['parameters']['setting']==name]
    for name,key,kind in [('ALLOWED_FILES','allowed_file',set),('ALLOWED_DIR_PATTERNS','allowed_dir_pattern',list),('DIRECT_EXECUTION_PATTERNS','direct_pattern',list),('FTS_MANAGER_DIRECT_EXECUTION_PATTERNS','fts_pattern',list)]:
        values=setting(key)
        if not values:raise ValueError('Native configuration omits '+name)
        suite.replace(module,name,kind(values))

    def allowed(path):
        result = rows('is_allowed_file', options=dict(argumentPath=str(path)))
        if len(result) != 1 or result[0]['code'] != 'IDE_BOOL':
            raise ValueError('One native allowlist Boolean is required')
        return bool(result[0]['parameters']['value'])

    def values(result):
        if any(row['code'] != 'IDE_ROW' for row in result):
            raise ValueError('Unexpected native direct-execution result')
        return [module.Violation(Path(r['parameters']['file']), r['parameters']['line_num'],
                                 r['parameters']['line'], r['parameters']['pattern'])
                for r in sorted(result, key=lambda r: (r['parameters']['file_order'], r['parameters']['phase'], r['parameters']['ordinal']))]

    def scan(entry, path, content=None):
        path = Path(path)
        try:
            source = path.read_text(encoding='utf-8')
        except (OSError, UnicodeDecodeError):
            source = ''
        options = dict(argumentPath=str(path), projectRoot=str(module.PROJECT_ROOT))
        if content is not None:
            options['callSource'] = content
        return values(rows(entry, [dict(path='input.py', source=source)], options))

    suite.replace(module, 'is_allowed_file', allowed)
    suite.replace(module, 'scan_file', lambda path: scan('scan_file', path))
    suite.replace(module, 'scan_source', lambda filepath, content, tree=None: scan('scan_source', filepath, content))
    suite.replace(module, 'scan_fts_manager_direct_execution', lambda path: scan('scan_fts_manager_direct_execution', path))
    suite.replace(module, 'scan_fts_manager_source', lambda filepath, content, tree=None: scan('scan_fts_manager_source', filepath, content))

    def directories(project_root, directories):
        # All roots enter one native demand batch; the policy discovers and selects files.
        env = ['ide.scan.'+str(i)+'='+str(Path(project_root)/directory) for i,directory in enumerate(directories)]
        return values(rows('scan_directories', options=dict(env=env, projectRoot=str(module.PROJECT_ROOT))))

    suite.replace(module, 'scan_directories', directories)
    suite.replace(module, 'scan_directory', lambda impl_dir: directories(Path(impl_dir).parent, [Path(impl_dir).name]))
