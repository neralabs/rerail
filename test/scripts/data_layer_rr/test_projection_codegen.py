"""A generated API policy must follow its canonical rule without manual divergence."""
import importlib.util
from pathlib import Path
import re
import unittest

ROOT=Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location('projection_codegen',ROOT/'scripts/generate/packs/data_layer_rr_projection.py')
codegen=importlib.util.module_from_spec(spec);spec.loader.exec_module(codegen)


class ProjectionGenerationTests(unittest.TestCase):
    def test_projection_stages_match_the_generator(self):
        self.assertEqual(codegen.projection_library(),(ROOT/'docs/rule-syntax/lib/data-layer/model-projection.rr').read_text())

    def test_api_policy_is_a_mechanical_derivative_with_its_own_identity(self):
        folder=ROOT/'gate/data-layer/packs/data-layer/candidates/rr'
        canonical=(folder/'serverplan-grammar-surface.rr').read_text()
        expected=re.sub(r'(?m)^\.policy .*$','.policy "data-layer-serverplan-grammar-surface-project" revision 1',canonical)+codegen.POLICY
        self.assertEqual(expected,(folder/'serverplan-grammar-surface-project.rr').read_text())


if __name__=='__main__':unittest.main()
