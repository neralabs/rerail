"""Permit passive builtin type declarations while rejecting linter execution."""
from pathlib import Path
import sys
import tempfile
import threading
import types
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from suite import Suite


class PassiveAliasGuard(unittest.TestCase):
    def check(self,source,*,allowed=False,shadow=False):
        with tempfile.TemporaryDirectory() as temporary:
            backend=Suite.__new__(Suite);backend.source=Path(temporary);backend.checks=[];backend.originals=[]
            backend.semantic_fallbacks=[];backend.host_protocol_calls=[];backend.item=None
            path=backend.source/'scripts/lint_example.py';path.parent.mkdir();path.write_text(source)
            module=types.ModuleType('scripts.lint_example');module.__file__=str(path)
            exec(compile(source,str(path),'exec'),module.__dict__)
            if shadow:
                class Shadow:
                    def __class_getitem__(cls,item):raise AssertionError('Shadowed builtin executed')
                module.list=Shadow
            try:
                backend.pytest_configure()
                if allowed:
                    value=module.Finding.__value__
                    self.assertEqual(value,dict[str,str|int|list[str]|None])
                else:
                    with self.assertRaisesRegex(AssertionError,'Unselected original linter'):
                        module.Finding.__value__ if hasattr(module.Finding,'__value__') else module.Finding()
            finally:sys.setprofile(None);threading.setprofile(None)
            if allowed:
                self.assertEqual(backend.semantic_fallbacks,[])
                calls=backend.passive_declaration_calls
                self.assertEqual(len(calls),1);self.assertEqual(calls[0]['declaration'],'Finding')
                self.assertEqual(calls[0]['kind'],'builtin-type-alias')
            else:self.assertTrue(backend.semantic_fallbacks)

    def test_builtin_container_union_alias(self):
        self.check('type Finding = dict[str, str | int | list[str] | None]\n',allowed=True)

    def test_future_annotation_alias(self):
        self.check('from __future__ import annotations\ntype Finding = dict[str, str | int | list[str] | None]\n',allowed=True)

    def test_alias_call_is_not_passive(self):
        self.check('def detector():\n return []\ntype Finding = detector()\n')

    def test_alias_attribute_is_not_passive(self):
        self.check('import typing\ntype Finding = typing.Any\n')

    def test_shadowed_builtin_is_not_passive(self):
        self.check('type Finding = list[str]\n',shadow=True)

    def test_function_named_like_alias_is_not_passive(self):
        self.check('def Finding():\n return []\n')


if __name__=='__main__':unittest.main()
