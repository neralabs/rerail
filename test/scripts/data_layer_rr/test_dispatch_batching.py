"""Vector APIs must prepare uncached Findings in one batch and retain identity."""
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace
import sys
import unittest

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'scripts/dsl/data_layer_rr'))
from dispatch_port import install


class DispatchBatchingTests(unittest.TestCase):
    def test_unapproved_prepares_uncached_values_once_and_keeps_equal_objects(self):
        @dataclass(frozen=True)
        class Finding:
            code:str
            path:Path=Path('a.py')
            line:int=1
            scope:str='f'
            expression:str='x'
            message:str='guidance'
            occurrence:int=1
            def render(self,root=Path('.')):raise AssertionError('Original formatter executed')
        class Backend:
            def __init__(self):self.calls=[]
            def replace(self,owner,name,value):setattr(owner,name,value)
            def native(self,check,entry,files,options=None):
                self.calls.append((entry,files,options))
                if entry=='_finding_projection':
                    return [dict(code='SD_FINDING',parameters=dict(ordinal=i,inventory=code,rendered='unused'))
                            for i,code in enumerate(options['dispatchFindingCode'])]
                if entry=='unapproved_findings':
                    return [dict(code='DISPATCH_UNAPPROVED',parameters=dict(ordinal=i)) for i in [2,0]]
                raise AssertionError('Unexpected native request')
        backend=Backend();module=SimpleNamespace(Finding=Finding,ROOT=Path('.'))
        install(backend,module)
        first,middle,last=Finding('A'),Finding('B'),Finding('A')
        result=module.unapproved_findings((first,middle,last),frozenset({'B'}))
        self.assertEqual(1,sum(entry=='_finding_projection' for entry,_,_ in backend.calls))
        self.assertEqual(2,len(backend.calls))
        self.assertEqual(['A','B','A'],backend.calls[-1][2]['dispatchInventory'])
        self.assertEqual(['B'],backend.calls[-1][2]['dispatchApproved'])
        self.assertEqual(2,len(result));self.assertIs(first,result[0]);self.assertIs(last,result[1])
        streamed=module.unapproved_findings((value for value in [first,middle,last]),frozenset({'B'}))
        self.assertEqual(2,len(streamed));self.assertIs(first,streamed[0]);self.assertIs(last,streamed[1])


if __name__=='__main__':unittest.main()
