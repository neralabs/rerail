# SPDX-License-Identifier: Apache-2.0
"""Reject evidence that does not contain executed native comparisons."""
import copy,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import pytest
from data_layer_ten.evidence import verify

def pair():
 record=dict(test='case',check='graph_lazy_sql_plan',entry='_violations_for_sources',files=[],parameters={},expected=[])
 phases=[dict(phase=phase,outcome='passed') for phase in ['setup','call','teardown']]
 oracle=dict(cases={'case':phases},records=[record],sources={})
 report=dict(oracle=False,mode='snapshot',exitCode=0,semanticFallbacks=[],originalCalls={},cases=oracle['cases'],records=[record],sources={},calls=[dict(test='case',check=record['check'],entry=record['entry'],findings=0)],engineSha256='build')
 return oracle,copy.deepcopy(report)

def test_original_execution_cannot_be_certified_as_native():
 oracle,report=pair();report['oracle']=True
 with pytest.raises(ValueError,match='original execution'):verify(oracle,[report])

def test_native_observations_require_executed_call_traces():
 oracle,report=pair();report['calls']=[]
 with pytest.raises(ValueError,match='native call'):verify(oracle,[report])

def test_call_trace_result_count_must_match_the_recorded_result():
 oracle,report=pair();report['calls'][0]['findings']=1
 with pytest.raises(ValueError,match='native call'):verify(oracle,[report])

def test_snapshot_results_cannot_be_claimed_as_workspace_execution():
 oracle,report=pair()
 with pytest.raises(ValueError,match='API mode'):verify(oracle,[report],mode='workspace')

def test_report_for_an_old_policy_cannot_certify_the_current_policy():
 oracle,report=pair();report['policySha256']={'graph_lazy_sql_plan':'old'}
 with pytest.raises(ValueError,match='Changed policy'):verify(oracle,[report],policies={'graph_lazy_sql_plan':'current'})
