"""Frontend storage construction cannot supply missing native coercion or validation."""
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
import model_dtos as models
from projection_port import construct


class ProjectionStorageTests(unittest.TestCase):
    def test_storage_preserves_types_defaults_presence_and_duplicate_tuples(self):
        from pydantic import BaseModel
        with patch.object(BaseModel,'model_validate',side_effect=AssertionError('Python model validator ran')):
            value=construct(models.ServerPlanFileFacts,dict(path='a.py',classes=[dict(name='C',qualname='C',line=3,
                is_step_subclass=True,bases=['A','A'])],grammar_surface_hits=[]))
        self.assertEqual({'path','classes','grammar_surface_hits'},value.model_fields_set)
        self.assertIsInstance(value.classes,tuple)
        self.assertIsInstance(value.classes[0],models.ServerPlanClassFacts)
        self.assertEqual(('A','A'),value.classes[0].bases)
        self.assertIs(value.classes[0].is_step_subclass,True)
        self.assertEqual((),value.functions)
        self.assertIsNone(value.syntax_error)

    def test_primitive_casts_must_already_be_native(self):
        for line in ['3',True,3.0]:
            with self.subTest(line=line),self.assertRaises(AssertionError):
                construct(models.GrammarSurfaceHit,dict(kind='k',line=line,symbol='s',message='m'))
        with self.assertRaises(AssertionError):
            construct(models.ServerPlanClassFacts,dict(name='C',qualname='C',line=3,is_step_subclass='yes'))

    def test_missing_required_fields_are_not_repaired_by_storage(self):
        with self.assertRaises(AssertionError):construct(models.GrammarSurfaceHit,dict(kind='k'))


if __name__=='__main__':unittest.main()
