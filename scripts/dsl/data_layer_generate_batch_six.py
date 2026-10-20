#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate sixth-batch candidates from verified pinned source constants."""
from __future__ import annotations
import argparse,importlib,re,sys
from pathlib import Path
from data_layer_six.common import *
from data_layer_probe_record import verify_sources
from data_layer_batch_six import MODULES
PIN='2b1504adc4291daaf080b2e932962b134da1a69a'
p=argparse.ArgumentParser(description=__doc__)
for name in ('source','repo'):p.add_argument('--'+name,type=Path,required=True)
p.add_argument('--output',type=Path,default=Path('packs/data-layer/candidates/rules'))
p.add_argument('--independent-sql',action='store_true',help='Generate the previous independent quality SQL groups for comparison')
p.add_argument('--quality-all-sql',action='store_true',help='Experimental single quality SQL scope with explicit production source gates')
p.add_argument('--baseline-relations',action='store_true',help='Generate eager text and recursive SQL for matched comparisons')
a=p.parse_args();source=a.source.resolve();sys.path[:0]=[str(source),str(source/'scripts')];a.output.mkdir(parents=True,exist_ok=True)
modules={name:importlib.import_module(module) for name,module in MODULES.items()}
verify_sources(source,a.repo,PIN)
from scripts.lint_lib.discovery import data_layer_production_dirs
ROOTS=[v+'/**/*.py' for v in data_layer_production_dirs(source)]
EXCLUDE=['**/.git/**','**/.venv/**','**/venv/**','**/tmp/**','**/__pycache__/**','**/tests/**','**/test_*.py']
def W(name,clauses,utils=None,options=None,roots=None,exclude=None,mode='block'):
 write(a.output,name,clauses,PIN,roots or ROOTS,EXCLUDE if exclude is None else exclude,utils,options,mode,optimized_relations=name=='serverplan_quality' and not a.baseline_relations)

# Direct bypass calls are deduplicated by method and source line.
m=modules['serverplan_bypass'];clauses=[]
for method in sorted(m.BYPASS_METHODS):
 clauses.append(B('bypass-'+method,call([method]),method,['scan_file','scan_source'],{'path':'file.path','line':'capture.CALL.startLine'}))
 clauses[-1]['budget']['query']['evidence']['query']['ast']['rule']['all'].append(P('CALL'))
W('serverplan_bypass',clauses)

# Removed raw execution APIs: syntax checks and one whole-file marker finding.
m=modules['raw_execute_regression'];clauses=[]
entries=['_check_ast','scan_file_content']
clauses.append(C('removed-calls',A(K('call'),F('function',A(K('attribute'),F('attribute',names(m.BANNED_CALL_ATTRS))))),
 analyzer_code('removed_call','RER001'),entries))
owned=lambda rule:A(rule,I(A(K('function_definition'),P('SAME_OWNER')),K('function_definition')),eq('OWNER','SAME_OWNER'))
banned=A(K('call'),F('function',A(K('attribute'),F('attribute',names(m.BANNED_CALL_ATTRS)))))
raise_helper=A(K('call'),F('function',A(K('attribute'),F('attribute',RX(r'^_raise_')))))
stub=A({'filePath':{'regex':{'$option':'compatibilityStubFiles','escape':'regex','join':'|','template':'^(?:{})$'}}},
 F('body',H(owned(O(K('raise_statement'),raise_helper)),stop='end')),
 N(F('body',H(owned(banned),stop='end'))))
clauses.append(C('removed-definitions',A(K('function_definition'),P('OWNER'),F('name',names(m.BANNED_DEF_NAMES)),N(stub)),
 analyzer_code('removed_definition','RER001'),entries))
for i,marker in enumerate(sorted(m.MIXED_HELPER_TEXT)):
 clauses.append(C('mixed-marker-'+str(i),A(K('module'),RX(re.escape(marker))),analyzer_code('mixed_helper_text','RER001'),['_check_text','scan_file_content'],anchor='file.start'))
W('raw_execute_regression',clauses,options={'compatibilityStubFiles':{'type':'array','items':'string','default':sorted(m.COMPATIBILITY_STUB_FILES)}})

from data_layer_six.target import target_clauses
m=modules['serverplan_target_architecture']
W('serverplan_target_architecture',target_clauses(m),options={'requiredShape':{'type':'boolean','default':True}},roots=[p+'/**/*.py' for p in m._SCAN_DIRS])

from data_layer_six.authority import authority_clause
m=modules['execution_authority_bypass']
W('execution_authority_bypass',[authority_clause(m)],roots=['l[0-9][0-9][0-9][0-9]_*/**/*.py','impl/**/*.py','data_layer/**/*.py','conform/**/*.py','tests/**/*.py'],exclude=['**/.git/**','**/.venv/**','**/__pycache__/**','**/tmp/**'])
from data_layer_six.direct_execution import direct_execution_clause
W('impl_direct_execution',[direct_execution_clause(modules['impl_direct_execution'])],mode='log')
from data_layer_six.dispatch import dispatch_clauses
m=modules['serverplan_dispatch_architecture'];clauses,utils=dispatch_clauses(m)
W('serverplan_dispatch_architecture',clauses,utils=utils,roots=[p+'/**/*.py' for p in m._GENERIC_DIRS]+[m._BASE_SERVER_PLAN_FILE])
from data_layer_six.steps import step_clauses
from data_layer_six.step_regions import user_callable_utility,strict_module_utility
m=modules['serverplan_steps']
W('serverplan_steps',step_clauses(m),utils={'step-user-callable':user_callable_utility(m),'step-strict-module':strict_module_utility()},options={'strictModulesOnly':{'type':'boolean','default':True}},roots=[p.relative_to(source).as_posix()+'/**/*.py' for p in m.STEP_DIRS]+[p.relative_to(source).as_posix() for p in m.SERVERPLAN_EXTRA_SCAN_FILES])
from data_layer_six.legacy import legacy_clauses
m=modules['legacy_execution_surfaces']
W('legacy_execution_surfaces',legacy_clauses(m),roots=[p.relative_to(source).as_posix()+'/**/*.py' for p in m.legacy_execution_surfaces_plugin(source).file_scope.directories],exclude=[*['**/'+v+'/**' for v in sorted(m.SKIP_DIRS)],'**/tests/**','**/test/**','**/conform/**'])
from data_layer_six.quality import quality_clauses
from data_layer_six.conditional import conditional_clauses
m=modules['serverplan_quality']
W('serverplan_quality',quality_clauses(m,shared_sql=not a.independent_sql,share_all=a.quality_all_sql)+conditional_clauses(m),roots=['**/*.py'],exclude=['**/.git/**','**/.venv/**','**/venv/**','**/tmp/**','**/__pycache__/**'])

from data_layer_six.state import state_clauses
m=modules['serverplan_state_tripwires'];clauses,utils=state_clauses(m)
W('serverplan_state_tripwires',clauses,utils=utils,options={'sourceProvided':{'type':'boolean','default':True},'returnKindsHex':{'type':'string','default':json.dumps({name:sorted(kinds) for name,kinds in m._load_generated_return_kinds().items()},sort_keys=True,separators=(',',':')).encode().hex()}})
