"""Decode native-selected syntax exceptions without parsing source in the host."""
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
sys.path.insert(0,str(Path(__file__).absolute().parents[3]/'scripts/dsl/data_layer_rr'))
from composition_port import install


class CompositionExceptionTransportTests(unittest.TestCase):
    def test_native_exception_class_and_complete_arguments_are_preserved(self):
        for kind,args in [('SyntaxError',['invalid syntax',['/owner/semantic.py',1,12,'def broken(:\n',1,13]]),
                          ('IndentationError',['expected block',['/owner/semantic.py',2,1,'pass\n',2,5]]),
                          ('TabError',['inconsistent indentation',['/owner/semantic.py',3,1,'        pass\n',3,0]]),
                          ('SyntaxError',['source code string cannot contain null bytes'])]:
            with self.subTest(kind=kind):
                calls=[]
                class Backend:
                    def replace(self,owner,name,value):setattr(owner,name,value)
                    def native(self,*a,**kw):
                        calls.append((a,kw))
                        return [dict(code='SPC_SYNTAX',message=args[0],parameters=dict(exception_kind=kind,exception_args=json.dumps(args)))]
                module=SimpleNamespace();install(Backend(),module)
                with self.assertRaises(SyntaxError) as raised:module.scan_semantic_source(Path('/owner/semantic.py'),'def broken(:\n')
                expected=tuple(tuple(v) if isinstance(v,list) else v for v in args)
                self.assertEqual(type(raised.exception).__name__,kind)
                self.assertEqual(raised.exception.args,expected)
                self.assertEqual(calls[0][1]['options'],dict(compositionFilename='/owner/semantic.py'))
