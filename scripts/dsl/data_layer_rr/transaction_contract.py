"""Run unchanged transaction-closure assertions with recorded inputs and complete state."""
import argparse,ast,importlib,inspect,json,os,subprocess,sys,threading
from pathlib import Path
from types import SimpleNamespace
import pytest
from migration import ROOT,PIN,git,sha,write_json,compare
from dto import canonical
from ast_transport import columns

METHODS={'NonLocalStateDetector':['visit'],'TransactionClosureLinter':['visit'],
         'ClosureDefinitionFinder':['visit']}
FUNCTIONS=['lint_file','lint_paths','lint_source']


def syntax(tree):
    """Transport syntax and scalar fields; never classify a binding or a violation."""
    data=columns(tree);nodes=[];pending=[tree]
    while pending:
        node=pending.pop();nodes.append(node);pending.extend(reversed(list(ast.iter_child_nodes(node))))
    for key,attr in [('astColumns','col_offset'),('astArgNames','arg'),('astKeywords','arg'),('astAsNames','asname')]:
        data[key]=[str(getattr(n,attr,0)) if key=='astColumns' else str(getattr(n,attr,'') or '') for n in nodes]
    data.update(astListNodes=[],astListNames=[],astListOrdinals=[])
    for index,node in enumerate(nodes):
        for ordinal,name in enumerate(getattr(node,'names',[])):
            if isinstance(name,str):
                data['astListNodes'].append(str(index));data['astListNames'].append(name);data['astListOrdinals'].append(str(ordinal))
    return data


class Contract:
    def __init__(self,args):
        self.args=args;self.calls=[];self.cases={};self.collected=[];self.depth=0;self.item=None;self.originals=[];self.observed={}
        self.source=args.source.absolute();self.module=None;self.backend=None;self.semantic_fallbacks=[]
        self.baseline=json.loads(args.baseline.read_text()) if args.command=='native' else None
        if self.baseline:
            assert self.baseline['sourcePin']==PIN and self.baseline['exitCode']==0
            from suite import Suite
            self.backend=Suite(self.source,args.engine.absolute(),['transaction_closures'],0,1,args.baseline,
                               args.rule_root.absolute() if args.rule_root else None)
        self.harness={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),
            *[Path(__file__).with_name(n) for n in ['transaction_port.py','transaction_probe.py','ast_transport.py','replay.py','suite.py','dto.py']]]}

    def root(self):
        return self.item.funcargs.get('tmp_path') if self.item else None

    def value(self,v):
        if isinstance(v,ast.AST):return dict(syntax=syntax(v))
        if isinstance(v,set):return dict(kind='set',items=sorted(self.value(x) for x in v))
        if isinstance(v,list):return [self.value(x) for x in v]
        if isinstance(v,tuple):return dict(kind='tuple',items=[self.value(x) for x in v])
        if isinstance(v,dict):return dict(kind='dict',items=[[self.value(k),self.value(x)] for k,x in v.items()])
        if isinstance(v,str) and self.root() is not None:
            if v==str(self.root()):return '<fixture>'
            return v.replace(str(self.root())+'/', '<fixture>/')
        if hasattr(v,'__dataclass_fields__'):
            return dict(kind='dataclass',type=type(v).__module__+'.'+type(v).__qualname__,
                        fields={k:self.value(getattr(v,k)) for k in v.__dataclass_fields__})
        return canonical(v,self.root())

    def state(self,instance):
        return {k:self.value(v) for k,v in vars(instance).items()}

    def pytest_configure(self):
        self.module=importlib.import_module('scripts.lint_transaction_closures');m=self.module
        from native_guard import codes
        originals=codes({'transaction_closures':m})
        targets=[(m,n) for n in FUNCTIONS]+[(getattr(m,c),n) for c,names in METHODS.items() for n in names]
        signatures={(owner,name):inspect.signature(getattr(owner,name)) for owner,name in targets}
        if self.backend:
            from transaction_port import install
            install(self.backend,m)
        for owner,name in targets:
            original=getattr(owner,name);signature=signatures[owner,name];entry=name if owner is m else owner.__name__+'.'+name
            def wrap(original,signature,entry):
                def run(*args,**kwargs):
                    if self.depth:return original(*args,**kwargs)
                    values=dict(signature.bind(*args,**kwargs).arguments);instance=values.pop('self',None)
                    event=dict(test=self.item.nodeid,entry=entry,arguments={k:self.value(v) for k,v in values.items()})
                    if instance is not None:event['before']=self.state(instance)
                    self.depth+=1
                    try:
                        result=original(*args,**kwargs);event['return']=self.value(result)
                        if instance is not None:event['after']=self.state(instance)
                    finally:self.depth-=1
                    if self.baseline:
                        expected=[r for r in self.baseline['calls'] if r['test']==event['test']]
                        index=self.observed.get(event['test'],0);self.observed[event['test']]=index+1
                        compare(expected[index],event)
                    self.calls.append(event);return result
                return run
            self.originals.append((owner,name,original));setattr(owner,name,wrap(original,signature,entry))
        if self.backend:
            def guard(frame,event,arg):
                if event!='call' or frame.f_code not in originals:return
                _,name,passive=originals[frame.f_code]
                if not passive and name not in {'cli_main'}:
                    self.semantic_fallbacks.append(name);raise AssertionError('Original detector invoked: '+name)
            sys.setprofile(guard);threading.setprofile(guard)

    def pytest_collection_modifyitems(self,items):
        self.collected=[item.nodeid for item in items]
        if self.baseline:compare(self.baseline['collected'],self.collected)
        items[:]=items[self.args.part::self.args.parts]

    def pytest_runtest_setup(self,item):
        self.item=item
        if self.backend:self.backend.item=item

    def pytest_runtest_logreport(self,report):
        self.cases.setdefault(report.nodeid,[]).append(dict(phase=report.when,outcome=report.outcome))

    def pytest_unconfigure(self):
        sys.setprofile(None);threading.setprofile(None)
        for owner,name,original in reversed(self.originals):setattr(owner,name,original)

    def finish(self,code):
        if self.backend:
            self.backend.worker.stdin.close();self.backend.worker.wait(timeout=10);self.backend.cache.cleanup()
        selected=self.collected[self.args.part::self.args.parts]
        missing=[]
        if self.baseline:
            for test in selected:
                expected=sum(r['test']==test for r in self.baseline['calls'])
                if expected!=self.observed.get(test,0):missing.append(test)
        result=dict(format='rerail.transaction-closures-original-contracts/v1',sourcePin=PIN,commit=PIN,
            pytest_exit_code=int(code),exitCode=int(code),events=[],source_mismatches=[],oracle=self.backend is None,
            execution='native-snapshot' if self.backend else 'offline-source',fullMigrationAcceptance=False,
            part=self.args.part,parts=self.args.parts,collected=self.collected,cases=self.cases,calls=self.calls,
            semanticFallbacks=self.semantic_fallbacks,missingResults=missing,casesWithoutCalls=[t for t in selected if not any(r['test']==t for r in self.calls)],
            sourceSha256=sha((self.source/'scripts/lint_transaction_closures.py').read_bytes()),
            testsSha256=sha((self.source/'scripts/tests/test_lint_transaction_closures.py').read_bytes()),
            harnessSha256=sha(Path(__file__).read_bytes()))
        if self.backend:result.update(nativeCalls=self.backend.calls,engineSha256=sha(self.args.engine.read_bytes()))
        for name,digest in self.harness.items():assert sha((ROOT/name).read_bytes())==digest,'Harness changed during acceptance: '+name
        result['harnessFiles']=self.harness
        write_json(self.args.out,result)
        if missing and not code:raise AssertionError('Missing original API returns: '+repr(missing))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for key in ['source','out','baseline','engine','rule-root']:parser.add_argument('--'+key,type=Path,required=key in ['source','out'])
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    args.source=args.source.absolute();args.out=args.out.absolute()
    if args.baseline:args.baseline=args.baseline.absolute()
    if args.engine:args.engine=args.engine.absolute()
    if args.rule_root:args.rule_root=args.rule_root.absolute()
    assert git(args.source,'rev-parse','HEAD')==PIN
    for name in ['scripts/lint_transaction_closures.py','scripts/tests/test_lint_transaction_closures.py']:
        assert (args.source/name).read_bytes()==subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name])
    assert 0<=args.part<args.parts and (args.command=='native' or (args.part,args.parts)==(0,1))
    sys.path[:0]=[str(args.source),str(args.source/'scripts'),str(ROOT/'scripts/dsl/data_layer_audit')]
    os.chdir(args.source);contract=Contract(args)
    code=pytest.main(['-q','-p','no:cacheprovider','-o','addopts=','scripts/tests/test_lint_transaction_closures.py'],plugins=[contract])
    contract.finish(code);raise SystemExit(code)


if __name__=='__main__':main()
