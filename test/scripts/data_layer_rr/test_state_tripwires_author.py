"""Preserve recursive matcher definitions and relation identity during authoring."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/generate/packs'))
from data_layer_rr_state import namespace, specialize, source_messages


class StateTripwiresAuthorTests(unittest.TestCase):
    def test_local_relations_are_renamed_in_nearest_and_recursive_uses(self):
        source='''.decl kind(k: symbol)
.decl edge(n: node, p: node)
kind("kind").
edge(n,p) :- nearest(n,p,kind), @kind("kind") = 1.
// kind is a name in this comment, not a relation reference.
'''
        self.assertEqual(namespace(source,'local_'),'''.decl local_kind(k: symbol)
.decl local_edge(n: node, p: node)
local_kind("kind").
local_edge(n,p) :- nearest(n,p,local_kind), @kind("kind") = 1.
// kind is a name in this comment, not a relation reference.
''')

    def test_recursive_matchers_remain_references_and_options_choose_one_branch(self):
        utils={'tail':dict(any=[dict(kind='identifier'),dict(has=dict(matches='tail'))])}
        value=dict(all=[dict(matches='tail'),{'$option':'enabled','cases':{
            'true':dict(kind='call'),'false':dict(kind='identifier')}}])
        self.assertEqual(specialize(value,utils,dict(enabled=True)),
                         dict(all=[dict(matches='tail'),dict(kind='call')]))
        with self.assertRaisesRegex(ValueError,'Unknown matcher utility'):
            specialize(dict(matches='missing'),utils,{})

    def test_source_diagnostics_keep_adjacent_literals_and_interpolation_sites(self):
        with tempfile.TemporaryDirectory() as scratch:
            source=Path(scratch)/'linter.py'
            source.write_text('''
def scan():
    Violation(code="SPT-006", qualname=qualname,
              message=("Encountered " f"{display_name!r}; " "only SqlStepIR is permitted."))
    Violation(code="SPT-006", qualname=qualname,
              message=f"must not construct {call_name}")
''')
            self.assertEqual(source_messages(source),{
                3:('SPT-006','Encountered {detail}; only SqlStepIR is permitted.'),
                5:('SPT-006','must not construct {callee}')})
