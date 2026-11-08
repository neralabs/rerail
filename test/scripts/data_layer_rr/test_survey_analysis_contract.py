"""Source analysis comparisons retain categories, list order, multiplicity, and explicit refusals."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from survey_analysis_contract import native_result,same_result


class SurveyAnalysisContractTests(unittest.TestCase):
    def test_complete_result_keeps_category_order_and_symbol_multiplicity(self):
        def row(code,**parameters):return dict(code=code,parameters=parameters)
        rows=[row('IDC_GROUP',category='high',ordinal=1),row('IDC_GROUP',category='unused',ordinal=0),
              row('IDC_ITEM',category='unused',sid='last',ordinal=2),
              row('IDC_ITEM',category='unused',sid='first',ordinal=0),
              row('IDC_ITEM',category='unused',sid='first',ordinal=1)]
        actual=native_result(rows)
        expected=dict(unused=['first','first','last'],high=[])
        self.assertTrue(same_result(expected,actual))
        for changed in [dict(high=[],unused=['first','first','last']),
                        dict(unused=['first','last'],high=[]),
                        dict(unused=['last','first','first'],high=[]),
                        dict(unused=['first','first','last'])]:
            with self.subTest(changed=changed):self.assertFalse(same_result(expected,changed))

    def test_explicit_incomplete_diagnostic_cannot_be_rendered_as_a_clean_result(self):
        with self.assertRaisesRegex(ValueError,'IDC_SOURCE_GRAPH_REQUIRED'):
            native_result([dict(code='IDC_SOURCE_GRAPH_REQUIRED',parameters={})])
