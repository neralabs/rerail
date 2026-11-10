"""CLI protocol and passive I/O; native rules own dispatch decisions and text."""
import argparse
import json
from pathlib import Path
import sys
import time
from migration import ROOT,sha
from replay import execute_native,execute_workspace

POLICY=ROOT/'gate/data-layer/packs/data-layer/candidates/rr/serverplan-dispatch-architecture.rr'
_CONFIG={}


class Driver:
    def __init__(self,engine,mode,executions):self.engine=engine;self.mode=mode;self.executions=executions
    def native(self,entry,options=None,files=()):
        execute=execute_native if self.mode=='snapshot' else execute_workspace
        started=time.monotonic()
        value=execute(self.engine,POLICY,list(files),dict(entryPoint=entry,**(options or {})))
        value['requestTiming']=dict(entry=entry,seconds=round(time.monotonic()-started,3))
        self.executions.append(value);return value['diagnostics']
    def configuration(self):
        key=(str(self.engine),sha(POLICY.read_bytes()))
        if key not in _CONFIG:_CONFIG[key]=self.native('_dispatch_cli_configuration')
        rows=_CONFIG[key]
        if not rows or any(r['code'] not in {'DCLI_CONFIG','DCLI_ARG','DCLI_FLAG'} for r in rows):
            raise RuntimeError('Native dispatch CLI configuration is missing')
        return rows


def output(rows):
    known={'DCLI_OUTPUT','DCLI_ACTION','DCLI_EXIT','DCLI_COUNT','DCLI_INVENTORY'}
    if any(r['code'] not in known for r in rows):raise ValueError('Unexpected native CLI output')
    for p in sorted((r['parameters'] for r in rows if r['code']=='DCLI_OUTPUT'),key=lambda p:(p['phase'],p['ordinal'])):
        {'stdout':sys.stdout,'stderr':sys.stderr}[p['channel']].write(p['text'])


def one(rows,code,key):
    values=[r['parameters'][key] for r in rows if r['code']==code]
    if len(values)!=1:raise ValueError('One native '+code+' is required')
    return values[0]


def install(engine,module,mode,executions):
    driver=Driver(engine,mode,executions)
    default_inventory=module._load_inventory.__defaults__[0]
    render_root=module.Finding.render.__defaults__[0]
    def read_inventory(path):return path.read_text(encoding='utf-8') if path.exists() else ''
    def load_inventory(path=default_inventory):
        rows=driver.native('_dispatch_inventory',dict(dispatchInventoryText=read_inventory(path)))
        if any(r['code']!='DCLI_INVENTORY' for r in rows):raise ValueError('Unexpected native inventory output')
        return tuple(r['parameters']['value'] for r in sorted(rows,key=lambda r:r['parameters']['ordinal']))
    def scan_repository(root=module.scan_repository.__defaults__[0]):
        from dispatch_cli_workspace import scan
        return scan(driver,module,root)
    def cli_main():
        rows=driver.configuration();config={r['parameters']['key']:r['parameters']['value'] for r in rows if r['code']=='DCLI_CONFIG'}
        parser=argparse.ArgumentParser(description=config['description'])
        for arg in sorted((r['parameters'] for r in rows if r['code']=='DCLI_ARG'),key=lambda p:p['ordinal']):
            flags=[r['parameters']['value'] for r in sorted(rows,key=lambda r:r['parameters'].get('ordinal',0)) if r['code']=='DCLI_FLAG' and r['parameters']['argument']==arg['ordinal']]
            kwargs=dict(action=arg['action'])
            if arg['dest']:kwargs['dest']=arg['dest']
            if arg['has_help']:kwargs['help']=arg['help']
            parser.add_argument(*flags,**kwargs)
        args=parser.parse_args();inventory=module._load_inventory()
        options=dict(dispatchInventory=list(inventory),cliSign=str(int(args.sign_inventory)),cliCheck=str(int(args.check)),sourceRoot=str(module.ROOT),inventoryPath=str(module.INVENTORY_PATH))
        prelude=driver.native('_dispatch_cli_begin',options);output(prelude);action=one(prelude,'DCLI_ACTION','value')
        if action=='exit':return one(prelude,'DCLI_EXIT','status')
        if action=='sign':
            result=module.sign_and_update_allowlist(Path(module.__file__),config['linter'],[],config['signature_variable'])
            rows=driver.native('_dispatch_cli_signed',dict(cliSignResult=str(int(bool(result)))))
            output(rows);return one(rows,'DCLI_EXIT','status')
        if action!='scan':raise ValueError('Unexpected CLI action')
        findings=module.scan_repository()
        options.update({'dispatchFinding'+n.title():[str(getattr(f,n)) for f in findings] for n in ['code','path','line','scope','expression','message','occurrence']})
        options['dispatchFindingRoot']=[str(render_root)]*len(findings)
        rows=driver.native('_dispatch_cli_report',options);output(rows)
        if one(rows,'DCLI_ACTION','value')=='verify':
            result=module.verify_allowlist(config['linter'],[],module.INVENTORY_SIGNATURE)
            rows=driver.native('_dispatch_cli_verified',dict(cliVerifyResult=str(int(bool(result))),cliFindingCount=one(rows,'DCLI_COUNT','value')))
            output(rows)
        return one(rows,'DCLI_EXIT','status')
    def verify(linter,entries,signature):
        rows=driver.native('_dispatch_cli_verify',dict(cliVerifyLinter=linter,cliVerifyEntries=list(entries),cliVerifySignature=signature))
        values=[r['parameters']['ok'] for r in rows if r['code']=='DCLI_VERIFY']
        if len(values)!=1 or values[0] not in [0,1]:raise ValueError('One native verification result required')
        output([r for r in rows if r['code']!='DCLI_VERIFY']);return bool(values[0])
    def sign(script,linter,entries,variable_name,*,version=1):
        from replay import sign_native
        rows=driver.native('_dispatch_cli_sign_request',dict(cliSignLinter=linter,cliSignEntries=list(entries),cliSignVariable=variable_name))
        if len(rows)!=1 or rows[0]['code']!='DCLI_SIGN_REQUEST':raise ValueError('One native signing request required')
        request=rows[0]['parameters']
        replies=sign_native(driver.engine,[dict(id=request['request_id'],key_env=request['key_env'],message=request['message'])])
        if len(replies)!=1 or replies[0]['id']!=request['request_id']:raise ValueError('One matching native signing reply required')
        reply=replies[0]
        if reply['status']=='signed':
            rows=driver.native('_dispatch_cli_sign_update',dict(cliSignVariable=variable_name,scriptSource=script.read_text(encoding='utf-8'),scriptPath=str(script),newSignature=reply['signature']))
        else:rows=driver.native('_dispatch_cli_sign_error',dict(signStatus=reply['status'],signDetail=reply['detail']))
        if len(rows)!=1 or rows[0]['code']!='DCLI_SIGN_RESULT':raise ValueError('One native signing result required')
        value=rows[0]['parameters']
        if value['write_file']:script.write_text(value['updated_source'],encoding='utf-8')
        sys.stdout.write(value['stdout']);sys.stderr.write(value['stderr']);return bool(value['ok'])
    module._load_inventory=load_inventory;module.scan_repository=scan_repository;module.cli_main=cli_main
    module.verify_allowlist=verify;module.sign_and_update_allowlist=sign
