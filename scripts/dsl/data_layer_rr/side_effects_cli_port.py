"""Transport CLI arguments and findings; native relations decide output and status."""
import argparse
import builtins
import json
from pathlib import Path
import sys
from value_columns import columns

CHECK='plan_scope_side_effects'


def one(rows,code,key):
    values=[row['parameters'][key]for row in rows if row['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' required')
    return values[0]


def install(backend,module):
    config=backend.native(CHECK,'_pse_cli_configuration',[])
    schema=json.loads(one(config,'PSE_CLI_CONFIG','schema'))
    fields=one(config,'PSE_CLI_CONFIG','fields').split(',')
    def report(violations,*,verbose):
        values=[{key:getattr(value,key)for key in fields}for value in violations]
        rows=backend.native(CHECK,'_pse_print_report',[],dict(columns(values),pseVerbose=str(int(verbose))))
        if any(row['code']not in {'PSE_CLI_OUTPUT','PSE_CLI_ERROR'}for row in rows):raise ValueError('Unexpected native report response')
        for p in sorted((row['parameters']for row in rows if row['code']=='PSE_CLI_OUTPUT'),key=lambda p:(p['phase'],p['ordinal'])):
            sys.stdout.write(p['text'])
        errors=[row['parameters']for row in rows if row['code']=='PSE_CLI_ERROR']
        if errors:
            if len(errors)!=1:raise ValueError('One native report error required')
            raise getattr(builtins,errors[0]['error_type'])(errors[0]['detail'])
    def cli():
        parser=argparse.ArgumentParser(description=schema['description'])
        for argument in schema['arguments']:parser.add_argument(*argument['names'],**argument['kwargs'])
        args=parser.parse_args()
        rows=backend.native(CHECK,'_pse_cli_root',[],dict(pseCliSource=str(Path(module.__file__).resolve())))
        root=Path(one(rows,'PSE_CLI_ROOT','path'))
        violations=module.scan_repo(root)
        module._print_report(violations,verbose=args.verbose)
        rows=backend.native(CHECK,'_pse_cli_exit',[],dict(pseCheck=str(int(args.check)),pseSeverities=[value.severity for value in violations]))
        return one(rows,'PSE_CLI_EXIT','status')
    backend.replace(module,'_print_report',report)
    backend.replace(module,'cli_main',cli)
