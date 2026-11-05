#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Bind successful assertion runs, positive controls and implementation files."""
import argparse,hashlib,json
from pathlib import Path
from evidence import PIN,verify


def receipt(oracle_path,snapshots,workspaces,controls):
 oracle=json.loads(oracle_path.read_text())
 load=lambda paths:[json.loads(path.read_text()) for path in paths]
 paths={}
 for mode,reports in [('snapshot',load(snapshots)),('workspace',load(workspaces))]:
  assert all(report['mode']==mode for report in reports),'Mixed API modes'
  summary=verify(oracle,reports)
  summary['caseOutcomes']={case:phases for report in reports for case,phases in report['cases'].items()}
  summary['instrumentationTranslations']=[item for report in reports for item in report['instrumentationTranslations']]
  summary['semanticFallbacks']=[item for report in reports for item in report['semanticFallbacks']]
  summary['sourceFilesChecked']=[report['sourceFilesChecked'] for report in reports]
  paths[mode]=summary
 assert paths['snapshot']['engineSha256']==paths['workspace']['engineSha256']
 assert paths['snapshot']['policySha256']==paths['workspace']['policySha256']
 negative_reports=load(controls);negative=verify(oracle,negative_reports,True)
 assert negative['engineSha256']==paths['snapshot']['engineSha256']
 negative['failedCases']=sum(any(p['outcome']=='failed' for p in phases) for report in negative_reports for phases in report['cases'].values())
 negative['caseOutcomes']={case:phases for report in negative_reports for case,phases in report['cases'].items()}
 files=set(Path('scripts/dsl/data_layer_nine').rglob('*.py'))|set(Path('packs/data-layer/rules').glob('*.json'))
 files.update(Path(p) for p in [str(oracle_path),'conformance/data-layer/batch-nine.ts','conformance/data-layer/batch-nine-kernels.ts','conformance/data-layer/native-worker.ts','conformance/data-layer/execution.ts','conformance/engine.ts','test/conformance/data-layer/batch-nine.test.ts','test/engine/native/data-layer-nine/canonical-structure.test.ts','test/engine/native/data-layer-nine/parser-source.test.ts','test/conformance/data-layer/batch-nine-shape-audit.test.ts','fixtures/host/data-layer-port/batch-09-shape-audit.json','fixtures/host/data-layer-port/batch-09-third-audit-20261105.json','test/conformance/data-layer/batch-nine-public-contract.test.ts','fixtures/host/data-layer-port/batch-09-public-scope-oracle.json','fixtures/host/data-layer-port/batch-09-configured-defaults.json','fixtures/host/data-layer-port/batch-09-public-audit-20261105.json','test/conformance/data-layer/batch-nine-production-regressions.test.ts','test/conformance/data-layer/production-runtime.test.ts','packs/data-layer/runtime/observations.py','src/cli/data-layer-audit.ts','src/cli/cli.ts','src/cli/index.ts','src/engine/code-rule/code-rule-schema.ts','src/engine/code-rule/code-rule.ts','.github/workflows/ci.yml','scripts/generate/packs/data_layer_batch_nine.py','fixtures/host/data-layer-port/batch-09-kernels.json','scripts/dsl/data_layer_audit/native_suite.py','scripts/dsl/data_layer_audit/native_guard.py','scripts/dsl/data_layer_audit/native_scanners.py','scripts/dsl/data_layer_batch_eight.py','scripts/dsl/data_layer_probe_record.py','scripts/dsl/data_layer_generate_batch_seven.py','scripts/dsl/data_layer_six/common.py','scripts/dsl/data_layer_six/sql_columns.py','scripts/dsl/data_layer_eight/contracts.py','scripts/dsl/data_layer_eight/python_repr.py'])
 native=['compile/relational.rs','relational.rs','relational/producer.rs','relational/producer/layout.rs','relational/python_ast.rs','relational/shared.rs','relational/syntax.rs','value/python_ast.rs']
 files.update(Path('native/src/code_rule_dsl')/p for p in native)
 files.update(p for p in Path('native/src/code_rule_dsl/value/python_ast').rglob('*') if p.suffix in {'.py','.rs'})
 hashes={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files) if p.is_file()}
 return dict(format='rerail.data-layer-batch-nine-assertions/v1',sourcePin=PIN,engineSha256=paths['snapshot']['engineSha256'],executionPaths=paths,positiveControls=negative,fileSha256=hashes,coverage=dict(upstreamLinterFrameworkCases=sum(not name.startswith('::') and 'test_codemod_bootstrap_cache_health.py' not in name for name in oracle['cases']),adjacentHostCodemodCases=sum('test_codemod_bootstrap_cache_health.py' in name for name in oracle['cases']),additionalCases=sum(name.startswith('::') for name in oracle['cases']),recordedApiResults=len(oracle['records']),recordedPositiveResults=sum(isinstance(r['expected'],list) and bool(r['expected']) for r in oracle['records'])))


def main():
 p=argparse.ArgumentParser(description=__doc__)
 p.add_argument('--oracle',type=Path,required=True);p.add_argument('--snapshots',type=Path,nargs='+',required=True);p.add_argument('--workspaces',type=Path,nargs='+',required=True);p.add_argument('--controls',type=Path,nargs='+',required=True);p.add_argument('--output',type=Path,required=True)
 args=p.parse_args();args.output.write_text(json.dumps(receipt(args.oracle,args.snapshots,args.workspaces,args.controls),indent=2)+'\n')
if __name__=='__main__':main()
