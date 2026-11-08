"""Record complete factory-produced facts while running unchanged original collector tests."""
import argparse,importlib,json,os,subprocess,sys,threading
from pathlib import Path
import pytest
from migration import ROOT,PIN,git,sha,write_json,compare


class Contract:
    def __init__(self,args):
        self.args=args;self.item=None;self.events=[];self.cases={};self.collected=[];self.originals=[];self.semantic_fallbacks=[];self.observed={}
        self.baseline=json.loads(args.baseline.read_text()) if args.command=='native' else None;self.backend=None
        if self.baseline:
            from suite import Suite
            self.backend=Suite(args.source,args.engine,[args.check],0,1,args.baseline,args.rule_root)
        self.harness={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),
            *[Path(__file__).with_name(n) for n in ['callback_wrappers_port.py','transaction_contract.py','ast_transport.py','replay.py','suite.py','dto.py']]]}

    def pytest_configure(self):
        from scripts.lint_lib.ast_collectors import AstEventCollector
        from native_guard import codes
        module=importlib.import_module('scripts.lint_'+self.args.check);original_codes=codes({self.args.check:module})
        name='transaction_callback_wrapper_fact_extractors';factory=getattr(module,name)
        if self.backend:
            from callback_wrappers_port import install
            install(self.backend,module);factory=getattr(module,name)
        contract=self
        def wrapped(root):
            mapping=factory(root)
            def wrap(create):
                def construct(path,source):
                    collector=create(path,source)
                    class Capture(AstEventCollector):
                        def __dir__(self):return sorted(set(super().__dir__())|set(dir(collector)))
                        def __getattr__(self,key):return getattr(collector,key)
                        def start_file(self,*args):return collector.start_file(*args)
                        def on_node(self,*args):return collector.on_node(*args)
                        def on_exit_node(self,*args):return collector.on_exit_node(*args)
                        def finish_file(self):
                            facts=collector.finish_file();base=contract.item.funcargs.get('tmp_path')
                            logical=path.relative_to(base).as_posix() if base is not None and path.is_relative_to(base) else str(path)
                            event=dict(test=contract.item.nodeid,path=logical,source=source,facts=facts)
                            if contract.baseline:
                                previous=[e for e in contract.baseline['events'] if e['test']==event['test']]
                                index=contract.observed.get(event['test'],0);contract.observed[event['test']]=index+1
                                try:compare(previous[index],event)
                                except AssertionError:contract.events.append(event);raise
                            contract.events.append(event);return facts
                    return Capture()
                return construct
            return {key:wrap(create) for key,create in mapping.items()}
        self.originals.append((module,name,getattr(module,name)));setattr(module,name,wrapped)
        if self.backend:
            def guard(frame,event,arg):
                if event=='call' and frame.f_code in original_codes:
                    _,name,passive=original_codes[frame.f_code]
                    if not passive:self.semantic_fallbacks.append(name);raise AssertionError('Original detector invoked: '+name)
            sys.setprofile(guard);threading.setprofile(guard)

    def pytest_collection_modifyitems(self,items):
        self.collected=[i.nodeid for i in items]
        if self.baseline:compare(self.baseline['collected'],self.collected)
        items[:]=items[self.args.part::self.args.parts]

    def pytest_runtest_setup(self,item):
        self.item=item
        if self.backend:self.backend.item=item

    def pytest_runtest_logreport(self,report):
        self.cases.setdefault(report.nodeid,[]).append(dict(phase=report.when,outcome=report.outcome))

    def pytest_unconfigure(self):
        sys.setprofile(None);threading.setprofile(None)
        for module,name,original in reversed(self.originals):setattr(module,name,original)

    def finish(self,code):
        if self.backend:
            self.backend.worker.stdin.close();self.backend.worker.wait(timeout=10);self.backend.cache.cleanup()
        missing=[]
        if self.baseline:
            for identity in self.collected[self.args.part::self.args.parts]:
                if sum(e['test']==identity for e in self.baseline['events'])!=self.observed.get(identity,0):missing.append(identity)
        for name,digest in self.harness.items():assert sha((ROOT/name).read_bytes())==digest,'Harness changed during acceptance: '+name
        result=dict(format='rerail.collector-original-fact-contracts/v1',sourcePin=PIN,commit=PIN,pytest_exit_code=int(code),exitCode=int(code),
            source_mismatches=[],oracle=self.backend is None,fullMigrationAcceptance=False,check=self.args.check,collected=self.collected,cases=self.cases,
            part=self.args.part,parts=self.args.parts,events=self.events,missingResults=missing,semanticFallbacks=self.semantic_fallbacks,harnessFiles=self.harness)
        if self.backend:result.update(engineSha256=sha(self.args.engine.read_bytes()),nativeCalls=self.backend.calls)
        write_json(self.args.out,result)
        if missing and not code:raise AssertionError('Original fact observations missing: '+repr(missing))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    parser.add_argument('--check',required=True);parser.add_argument('--tests',required=True)
    for key in ['source','out','baseline','engine','rule-root']:parser.add_argument('--'+key,type=Path,required=key in ['source','out'])
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    for key in ['source','out','baseline','engine','rule_root']:
        if getattr(args,key) is not None:setattr(args,key,getattr(args,key).absolute())
    assert args.check=='transaction_callback_wrappers' and git(args.source,'rev-parse','HEAD')==PIN and 0<=args.part<args.parts
    for name in ['scripts/lint_'+args.check+'.py',args.tests]:
        assert (args.source/name).read_bytes()==subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name])
    sys.path[:0]=[str(args.source),str(args.source/'scripts'),str(ROOT/'scripts/dsl/data_layer_audit')];os.chdir(args.source)
    contract=Contract(args);code=pytest.main(['-q','-p','no:cacheprovider','-o','addopts=',args.tests],plugins=[contract]);contract.finish(code);raise SystemExit(code)


if __name__=='__main__':main()
