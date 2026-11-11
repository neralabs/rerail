"""State scanner transport preserves complete values and project file order."""
from dataclasses import dataclass
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from state_tripwires_port import scan, snapshot


@dataclass
class Finding:
    code: str
    file: Path
    line: int
    qualname: str
    message: str


class StateTripwiresPortTests(unittest.TestCase):
    def test_project_results_keep_input_file_order_and_all_fields(self):
        module=SimpleNamespace(ROOT=Path('/source'),Violation=Finding)
        def row(path,line,code,qual,text):
            return dict(path=path,line=line,code=code,message=text,parameters=dict(qualname=qual,mode='sorted'))
        calls=[]
        def native(*args):
            calls.append(args)
            return [row('a.py',2,'SPT-028','materialize','exact second message'),
                    row('z.py',6,'SPT-028','Step.compile','exact first message')]
        arguments=dict(file_sources=[(Path('z.py'),'z'),(Path('a.py'),'a')])
        actual=scan(SimpleNamespace(native=native),module,arguments,'scan_project_sources')
        self.assertEqual(actual,[Finding('SPT-028',Path('z.py'),6,'Step.compile','exact first message'),
                                 Finding('SPT-028',Path('a.py'),2,'materialize','exact second message')])
        self.assertEqual(calls[0][2],[dict(path='z.py',source='z'),dict(path='a.py',source='a')])

    def test_absolute_caller_path_and_explicit_empty_return_map_survive_transport(self):
        module=SimpleNamespace(ROOT=Path('/source'),Violation=Finding)
        calls=[]
        def native(*args):
            calls.append(args)
            return [dict(path='elsewhere/impl/a.py',line=4,code='SPT-027',message='work',
                         parameters=dict(qualname='Step.compile',mode='sorted'))]
        arguments=dict(path=Path('/elsewhere/impl/a.py'),source='native()',project_return_kinds={})
        actual=scan(SimpleNamespace(native=native),module,arguments,'scan_source')
        self.assertEqual(actual,[Finding('SPT-027',Path('/elsewhere/impl/a.py'),4,'Step.compile','work')])
        self.assertEqual(calls[0][3],dict(returnKindNames=[],returnKindValues=[]))
        self.assertIsNone(snapshot(module,arguments,'scan_source')[2])

    def test_missing_source_is_not_silently_reconstructed(self):
        with self.assertRaisesRegex(ValueError,'original source'):
            snapshot(SimpleNamespace(ROOT=Path('/source')),dict(path=Path('a.py'),tree=object()),'scan_tree')
