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
        native_values=kind(values)
        if kind is set:
            # The host set's iteration order is not a rule contract. Keep its exact
            # object for original caller fixtures after validating all native values.
            if native_values!=getattr(module,name):raise ValueError('Native settings differ from caller metadata: '+name)
        else:suite.replace(module,name,native_values)
    signatures=setting('signature')
    if len(signatures)!=1:raise ValueError('One native allowlist signature is required')
    suite.replace(module,'_ALLOWED_FILES_SIGNATURE',signatures[0])

    def allowed(filepath):
        result = rows('is_allowed_file', options=dict(argumentPath=str(filepath)))
        if len(result) != 1 or result[0]['code'] != 'IDE_BOOL':
            raise ValueError('One native allowlist Boolean is required')
        return bool(result[0]['parameters']['value'])

    def values(result):
        if any(row['code'] != 'IDE_ROW' for row in result):
            raise ValueError('Unexpected native direct-execution result')
        return [module.Violation(Path(r['parameters']['file']), r['parameters']['line_num'],
                                 r['parameters']['line'], r['parameters']['pattern'])
                for r in sorted(result, key=lambda r: (r['parameters']['request_order'], r['parameters']['file_order'], r['parameters']['phase'], r['parameters']['ordinal']))]

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
    suite.replace(module, 'scan_file', lambda filepath: scan('scan_file', filepath))
    suite.replace(module, 'scan_source', lambda filepath, content, tree=None: scan('scan_source', filepath, content))
    suite.replace(module, 'scan_fts_manager_direct_execution', lambda filepath: scan('scan_fts_manager_direct_execution', filepath))
    suite.replace(module, 'scan_fts_manager_source', lambda filepath, content, tree=None: scan('scan_fts_manager_source', filepath, content))

    def directories(project_root, directories):
        # All roots enter one native demand batch; the policy discovers and selects files.
        env = ['ide.scan.'+str(i)+'='+str(Path(project_root)/directory) for i,directory in enumerate(directories)]
        return values(rows('scan_directories', options=dict(env=env, projectRoot=str(module.PROJECT_ROOT))))

    suite.replace(module, 'scan_directories', directories)
    suite.replace(module, 'scan_directory', lambda impl_dir: directories(Path(impl_dir).parent, [Path(impl_dir).name]))

    import argparse
    import sys

    def verification(linter_name, entries, signature_b64, *, version=1, warn_only=False):
        if version != 1 or warn_only:
            raise NotImplementedError('The direct-execution CLI requests strict version-one verification')
        result=rows('_verify_allowlist', options=dict(linterName=linter_name,allowlistEntries=list(entries),signature=signature_b64))
        if len(result)!=1 or result[0]['code']!='IDE_VERIFY':
            raise ValueError('One native verification result is required')
        value=result[0]['parameters'];sys.stderr.write(value['stderr'])
        return bool(value['value'])

    def pending_ratchet(*args, **kwargs):
        raise NotImplementedError('Native direct-execution ratchet actions are not yet ported')

    suite.replace(module.lint_signing,'verify_allowlist',verification)
    suite.replace(module.lint_ratchet,'check_ratchet',pending_ratchet)
    suite.replace(module.lint_ratchet,'update_ratchet_baseline',pending_ratchet)

    def cli_report(entry, root, violations=(), verbose=False):
        result=rows(entry, options=dict(projectRoot=str(root),cliVerbose=str(int(verbose)),
            reportPaths=[str(v.file) for v in violations],reportLines=[str(v.line_num) for v in violations],reportTexts=[v.line for v in violations]))
        if any(r['code'] not in {'IDE_OUTPUT','IDE_EXIT'} for r in result):
            raise ValueError('Unexpected native CLI response')
        exits=[r['parameters'] for r in result if r['code']=='IDE_EXIT']
        if len(exits)!=1:raise ValueError('One native CLI exit is required')
        for r in sorted((r['parameters'] for r in result if r['code']=='IDE_OUTPUT'),key=lambda r:(r['phase'],r['path_order'],r['ordinal'])):
            sys.stdout.write(r['text'])
        return exits[0]

    def cli_main():
        parser=argparse.ArgumentParser(description='Lint for direct database execution outside ServerPlan')
        parser.add_argument('--check',action='store_true',help='Exit with error code if violations found')
        parser.add_argument('--verbose',action='store_true',help='Show all matched lines')
        parser.add_argument('--sign-allowlist',action='store_true',help='Sign the ALLOWED_FILES list with NERA_SIGNING_KEY')
        parser.add_argument('--update-baseline',action='store_true',help='Update ratchet baseline with current counts')
        parser.add_argument('--sign',action='store_true',help='Sign baseline with NERA_SIGNING_KEY')
        parser.add_argument('--reason',type=str,default='',help='Approval reason for baseline update')
        parser.add_argument('--skip-signature-check',action='store_true',help='Skip baseline signature verification (remove once baseline is signed)')
        parser.add_argument('--require-baseline',action='store_true',help='Require an existing ratchet baseline in CI mode')
        args=parser.parse_args()
        if args.sign_allowlist:
            raise NotImplementedError('Native direct-execution allowlist signing is not yet ported')
        if not module.lint_signing.verify_allowlist('lint_impl_direct_execution',sorted(module.ALLOWED_FILES),module._ALLOWED_FILES_SIGNATURE):
            return cli_report('_cli_signature_failure',Path(module.__file__).parent.parent)['status']
        root=Path(module.__file__).parent.parent
        result=rows('_cli_paths',options=dict(env=['ide.cli.root='+str(root)]))
        if any(r['code']!='IDE_DIRECTORY' for r in result):raise ValueError('Unexpected native directory response')
        dirs=[r['parameters']['path'] for r in sorted(result,key=lambda r:(r['parameters']['priority'],r['parameters']['path']))]
        if not dirs:return cli_report('_cli_missing',root)['status']
        violations=module.scan_directories(root,dirs)
        report=cli_report('_cli_format',root,violations,args.verbose)
        counts={'violations':report['count']}
        if args.update_baseline:
            module.lint_ratchet.update_ratchet_baseline('lint_impl_direct_execution',counts,sign=args.sign,reason=args.reason)
            return 0
        if args.check:
            return 0 if module.lint_ratchet.check_ratchet('lint_impl_direct_execution',counts,require_signature=not args.skip_signature_check) else 1
        return report['status']

    suite.replace(module,'cli_main',cli_main)
