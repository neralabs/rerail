"""Keep independent callback occurrences and native API routing observable."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[3]/'scripts/dsl/data_layer_rr'))
from policy_routes import policy_filename
from serverplan_steps_port import legacy_rows


class LegacyTransportTests(unittest.TestCase):
    def test_order_and_identical_findings_keep_each_source_occurrence(self):
        def row(phase,call,node,root=0,path='a.py'):
            return dict(code='SSP_LEGACY',parameters=dict(root_order=root,path_order=path,
                phase=phase,call_order=call,keyword_order=0,node_order=node,priority=0,
                violation_code='LEGACY-POSTOP',file_path=path,line_number=3,category='same'))
        first=row(0,10,4);first['parameters']['category']='phase zero first'
        values=legacy_rows([row(1,0,4),first,row(1,0,5),row(0,0,4,1,'b.py')])
        self.assertEqual(['a.py','a.py','a.py','b.py'],[v['file'] for v in values])
        self.assertEqual('phase zero first',values[0]['message'])
        self.assertEqual([dict(code='LEGACY-POSTOP',file='a.py',line=3,message='same')]*2,values[1:3])

    def test_incompatible_native_rows_are_rejected(self):
        with self.assertRaisesRegex(ValueError,'Unexpected native legacy'):
            legacy_rows([dict(code='SSP_ROW')])

    def test_each_api_has_a_static_consumer_independent_of_expected_findings(self):
        for entry,policy in [('scan_legacy_postop_violations','legacy'),('_cli_paths','paths'),
                             ('_cli_report','report'),('_cli_legacy_report','report'),
                             ('_cli_signature_failure','report')]:
            with self.subTest(entry=entry):
                self.assertEqual('serverplan-steps-'+policy+'.rr',policy_filename('serverplan_steps',entry))
        self.assertEqual('serverplan-steps.rr',policy_filename('serverplan_steps','_cli_scan'))


if __name__ == '__main__':unittest.main()
