"""Capture complete public API values from unchanged shared MCP assertions."""
from collections import Counter
from functools import wraps
from dto import canonical
from validate_supplied_facts_checkpoint import require,equal

ENTRIES={
    'plan_composition_architecture':['scan_semantic_source','run_behavior_probes'],
    'dynamic_module_loading':['find_violations'],
    'no_source_inspection':['find_violations'],
    'sql_grammar_escape_hatches':['scan_usages_source'],
    'sync_async_signatures':['analyze_file'],
    'transaction_patterns':['analyze_file'],
}


def validate(expected,actual):
    def key(row):return row['test'],row['check'],row['entry'],row['ordinal']
    require(Counter(map(key,expected))==Counter(map(key,actual)),'Shared API observations omitted or repeated')
    wanted={key(row):row for row in expected}
    for row in actual:require(equal(row,wanted[key(row)]),'Shared API arguments or complete result differ')


class Observer:
    def __init__(self,modules):
        self.modules=modules;self.item=None;self.rows=[];self.counts=Counter();self.originals=[]

    def install(self):
        for check,module in self.modules.items():
            for entry in ENTRIES[check]:
                original=getattr(module,entry)
                def wrapper(check,entry,original):
                    @wraps(original)
                    def run(*args,**kwargs):
                        if self.item is None:return original(*args,**kwargs)
                        root=self.item.funcargs.get('tmp_path');key=(self.item.nodeid,check,entry)
                        row=dict(test=key[0],check=check,entry=entry,ordinal=self.counts[key],
                                 args=canonical(args,root),kwargs=canonical(kwargs,root))
                        self.counts[key]+=1
                        result=original(*args,**kwargs)
                        row['result']=canonical(result,root);self.rows.append(row)
                        return result
                    return run
                self.originals.append((module,entry,original));setattr(module,entry,wrapper(check,entry,original))

    def restore(self):
        for module,entry,original in reversed(self.originals):setattr(module,entry,original)
