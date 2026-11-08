"""Run every original transaction-pattern assertion and compare complete API observations."""
import argparse, ast, hashlib, importlib, inspect, os, subprocess, sys, threading
from pathlib import Path
import pytest
from migration import ROOT, PIN, git, sha, write_json, read_json, compare
from dto import canonical
from callback_wrappers_port import ast_input

FUNCTIONS = ['analyze_file', 'analyze_source', '_facts_for_files', '_collect_current_findings',
    '_infer_category', '_is_connection_class_init', '_get_categorized_functions',
    '_compute_signature_message', '_verify_signature', '_sign_categories', '_load_baseline',
    '_check_for_internal_transactions', '_get_public_key_bytes', '_baseline_cache_token', '_root_for_analysis_file']
METHODS = {'ConnectionParameterFinder': ['visit'], 'InternalTransactionFinder': ['visit', '_check_call']}


class Contract:
    def __init__(self, args):
        self.args=args; self.originals=[]; self.calls=[]; self.cases={}; self.collected=[]
        self.depth=0; self.item=None; self.observed={}; self.semantic_fallbacks=[]; self.backend=None
        self.baseline=read_json(args.baseline) if args.command=='native' else None
        if self.baseline:
            from suite import Suite
            self.backend=Suite(args.source,args.engine,['transaction_patterns'],args.part,args.parts,args.baseline,args.rule_root)
        self.harness={str(p.relative_to(ROOT)):sha(p.read_bytes()) for p in [Path(__file__),
            *[Path(__file__).with_name(n) for n in ['ast_transport.py','callback_wrappers_port.py','replay.py','suite.py','dto.py']]]}

    def value(self, value):
        if isinstance(value, ast.AST):return dict(syntax=ast_input(value)[0])
        if isinstance(value, set):return dict(kind='set',items=sorted(self.value(x) for x in value))
        if isinstance(value, list):return [self.value(x) for x in value]
        if isinstance(value, dict):return dict(kind='dict',items=[[self.value(k),self.value(v)] for k,v in value.items()])
        if isinstance(value, tuple):return dict(kind='tuple',items=[self.value(x) for x in value])
        root=self.item.funcargs.get('tmp_path') if self.item else None
        return canonical(value,root)

    def pytest_configure(self):
        import nacl.signing
        self.originals.append((nacl.signing.SigningKey,'generate',nacl.signing.SigningKey.__dict__['generate']))
        # Only ephemeral test keys: source and native runs get identical fixture signatures.
        nacl.signing.SigningKey.generate=classmethod(lambda cls:cls(hashlib.sha256(b'rerail transaction-pattern test fixture').digest()))
        self.module=importlib.import_module('scripts.lint_transaction_patterns');module=self.module
        from native_guard import codes
        guarded=codes({'transaction_patterns':module})
        targets=[(module,name) for name in FUNCTIONS]+[(getattr(module,c),name) for c,names in METHODS.items() for name in names]
        signatures={(owner,name):inspect.signature(getattr(owner,name)) for owner,name in targets}
        if self.backend:
            from patterns_port import install
            install(self.backend,module)
        for owner,name in targets:
            original=getattr(owner,name);signature=signatures[owner,name];entry=name if owner is module else owner.__name__+'.'+name
            def wrap(original,signature,entry):
                def run(*args,**kwargs):
                    if self.depth:return original(*args,**kwargs)
                    values=dict(signature.bind(*args,**kwargs).arguments);instance=values.pop('self',None)
                    arguments={k:self.value(v) for k,v in values.items() if k!='private_key_bytes'}
                    if 'private_key_bytes' in values:arguments['fixtureKeySha256']=sha(values['private_key_bytes'])
                    event=dict(test=self.item.nodeid,entry=entry,arguments=arguments)
                    if instance is not None:event['before']=self.value(vars(instance))
                    self.depth+=1
                    try:
                        result=original(*args,**kwargs);event['return']=self.value(result)
                        return result
                    except Exception as error:
                        event['error']=dict(type=type(error).__module__+'.'+type(error).__qualname__,args=self.value(error.args))
                        raise
                    finally:
                        self.depth-=1
                        if instance is not None:event['after']=self.value(vars(instance))
                        self.calls.append(event)
                        if self.baseline:
                            expected=[r for r in self.baseline['calls'] if r['test']==event['test']]
                            index=self.observed.get(event['test'],0);self.observed[event['test']]=index+1
                            compare(expected[index],event)
                return run
            self.originals.append((owner,name,original));setattr(owner,name,wrap(original,signature,entry))
        if self.backend:
            def guard(frame,event,arg):
                if event=='call' and frame.f_code in guarded:
                    _,name,passive=guarded[frame.f_code]
                    # Signing creates ephemeral fixtures outside policy execution. Verification and canonicalization are native.
                    if not passive and name!='_sign_categories':
                        self.semantic_fallbacks.append(name);raise AssertionError('Original detector/helper invoked: '+name)
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
        for owner,name,original in reversed(self.originals):setattr(owner,name,original)

    def finish(self,code):
        if self.backend:
            self.backend.worker.stdin.close();self.backend.worker.wait(timeout=10);self.backend.cache.cleanup()
        selected=self.collected[self.args.part::self.args.parts]
        missing=[t for t in selected if self.baseline and sum(r['test']==t for r in self.baseline['calls'])!=self.observed.get(t,0)]
        for name,digest in self.harness.items():assert sha((ROOT/name).read_bytes())==digest,'Harness changed: '+name
        result=dict(format='rerail.transaction-patterns-original-contracts/v1',sourcePin=PIN,commit=PIN,
            pytest_exit_code=int(code),exitCode=int(code),events=[],source_mismatches=[],oracle=self.backend is None,
            fullMigrationAcceptance=False,part=self.args.part,parts=self.args.parts,collected=self.collected,
            cases=self.cases,calls=self.calls,missingResults=missing,semanticFallbacks=self.semantic_fallbacks,
            casesWithoutCalls=[t for t in selected if not any(r['test']==t for r in self.calls)],harnessFiles=self.harness,
            sourceFiles={name:sha((self.args.source/name).read_bytes()) for name in
                ['scripts/lint_transaction_patterns.py','scripts/tests/test_lint_transaction_patterns.py']})
        if self.backend:result.update(engineSha256=sha(self.args.engine.read_bytes()),nativeCalls=self.backend.calls)
        write_json(self.args.out,result)
        if missing and not code:raise AssertionError('Missing complete API observations: '+repr(missing))


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['source','native'])
    for name in ['source','out','baseline','engine','rule-root']:parser.add_argument('--'+name,type=Path,required=name in ['source','out'])
    parser.add_argument('--part',type=int,default=0);parser.add_argument('--parts',type=int,default=1);args=parser.parse_args()
    for name in ['source','out','baseline','engine','rule_root']:
        if getattr(args,name) is not None:setattr(args,name,getattr(args,name).absolute())
    assert git(args.source,'rev-parse','HEAD')==PIN and 0<=args.part<args.parts
    for name in ['scripts/lint_transaction_patterns.py','scripts/tests/test_lint_transaction_patterns.py']:
        assert (args.source/name).read_bytes()==subprocess.check_output(['git','-C',str(args.source),'show',PIN+':'+name])
    sys.path[:0]=[str(args.source),str(args.source/'scripts'),str(ROOT/'scripts/dsl/data_layer_audit')];os.chdir(args.source)
    contract=Contract(args);code=pytest.main(['-q','-p','no:cacheprovider','-o','addopts=','scripts/tests/test_lint_transaction_patterns.py'],plugins=[contract]);contract.finish(code);raise SystemExit(code)


if __name__=='__main__':main()
