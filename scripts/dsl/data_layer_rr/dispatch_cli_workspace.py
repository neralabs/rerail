"""Run a production workspace or its complete file-buffer snapshot at a caller root."""
import json
from pathlib import Path
import subprocess
import tempfile
import time
from migration import ROOT,MANIFEST,read_json,verify_source_cohort,verify_engine_artifact,write_json
from replay import program_inputs


def scan(driver,module,root):
    started=time.monotonic()
    from dispatch_cli_port import POLICY
    source=read_json(ROOT/MANIFEST)['engineSource'];verify_source_cohort(ROOT,source);verify_engine_artifact(ROOT,source,driver.engine)
    # Load requests read exports, before the runtime option relation exists.
    # This scan entry does not execute the plugin's inventory reporter.
    options=dict(entryPoint='_dispatch_scan_repository',env=['data-layer.dispatch.scan-only=true'])
    if driver.mode=='snapshot':options['inputFiles']=[p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()]
    with tempfile.TemporaryDirectory(prefix='dispatch-cli-query-') as temporary:
        base=Path(temporary);write_json(base/'options.json',options)
        run=subprocess.run([str(driver.engine),'run-rr',str(POLICY),str(root),str(base/'out'),str(base/'options.json')],capture_output=True,text=True,timeout=100)
        if run.returncode:raise RuntimeError(run.stderr)
        result=json.loads(run.stdout);result['programInputs']=program_inputs(result['programInputs'])
    result['requestTiming']=dict(entry='_dispatch_scan_repository',seconds=round(time.monotonic()-started,3))
    driver.executions.append(result)
    rows=result['diagnostics']
    if any(r['code']!='DCLI_SCAN_FACT' for r in rows):raise ValueError('Unexpected native repository scan output')
    values=sorted((r['parameters'] for r in rows),key=lambda p:(p['path'],p['line'],p['source_code'],p['expression'],p['visit_order'],p['layer']))
    return tuple(module.Finding(p['source_code'],Path(p['path']),p['line'],p['scope'],p['expression'],p['message'],p['occurrence']) for p in values)
