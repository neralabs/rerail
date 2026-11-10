"""Transport blanket-bypass CLI arguments; native rules format and enforce results."""
from pathlib import Path


def install(suite,module):
    import argparse
    import sys
    check='no_blanket_grammar_bypass'
    settings=suite.native(check,'_configuration',[])
    if not settings or any(r['code']!='NGB_CONFIG' for r in settings):raise ValueError('Native bypass configuration is missing')
    def values(key):return [r['parameters']['value'] for r in settings if r['parameters']['key']==key]
    for name,key in [('ALLOWED_FILES','allowed'),('FORBIDDEN_PUBLIC_METHODS','method')]:
        if set(values(key))!=getattr(module,name):raise ValueError('Native bypass configuration differs: '+name)
    if values('sql')!=[module.SQL_CONNECTION_FILE]:raise ValueError('Native SQL connection path differs')
    suite.configuration_bindings.setdefault(check,dict(nativeCall=len(suite.calls)-1,names=['ALLOWED_FILES','FORBIDDEN_PUBLIC_METHODS','SQL_CONNECTION_FILE']))
    def cli_main():
        parser=argparse.ArgumentParser(description=values('description')[0])
        parser.add_argument('--check',action='store_true',help=values('check_help')[0]);args=parser.parse_args()
        root=Path(module.__file__).parent.parent
        bypass=module.check_no_bypass_references(root)
        methods=module.check_no_public_grammar_methods(root)
        rows=suite.native(check,'_cli_report',[],dict(bypassRows=bypass,methodRows=methods,checkMode=str(int(args.check))))
        if any(r['code'] not in {'NGB_OUTPUT','NGB_EXIT'} for r in rows):raise ValueError('Unexpected native bypass CLI output')
        for r in sorted((r['parameters'] for r in rows if r['code']=='NGB_OUTPUT'),key=lambda r:(r['phase'],r['ordinal'])):
            sys.stdout.write(r['stdout']);sys.stderr.write(r['stderr'])
        exits=[r['parameters']['value'] for r in rows if r['code']=='NGB_EXIT']
        if len(exits)!=1:raise ValueError('One native bypass exit value is required')
        return exits[0]
    suite.replace(module,'cli_main',cli_main)
