# SPDX-License-Identifier: Apache-2.0
"""Collect live plan-builder observations. SQLite makes all lint decisions."""
import json
from dataclasses import asdict,is_dataclass

def observations():
 from impl.common.l2070_server_plan.steps.collect_results import StaticValueAfterDepsStep
 from l2070_batching.semantic_plan import PlanGenerationId,PlanNamespaceId,SemanticPlanBuilder
 def builder(namespace,generation):return SemanticPlanBuilder(namespace=PlanNamespaceId(namespace),generation=PlanGenerationId(generation))
 def fingerprint():
  b=builder('composition-linter','probe');r=b.allocate_ref(kind='step');b.add_step(StaticValueAfterDepsStep(r,1,[]));b.add_return('value',r);return b.freeze().fingerprint
 fingerprints=[fingerprint(),fingerprint()]
 source=builder('probe-source','first');ref=source.allocate_ref(kind='step');source.add_step(StaticValueAfterDepsStep(ref,1,[]));source.add_return('value',ref);fragment=source.freeze().as_fragment()
 destination=builder('probe-destination','first');first=destination.add_fragment(fragment);second=destination.add_fragment(fragment);destination.freeze()
 def key(ref):return json.dumps(asdict(ref) if is_dataclass(ref) else ref,sort_keys=True,separators=(',',':'))
 return dict(fingerprints=fingerprints,firstRefs=[key(v) for v in first.ref_map.values()],secondRefs=[key(v) for v in second.ref_map.values()])


if __name__=='__main__':
 import contextlib,os,sys,tempfile
 from pathlib import Path
 root=Path(sys.argv[1]).resolve(strict=True)
 os.chdir(root);sys.path.insert(0,str(root))
 with tempfile.TemporaryDirectory(prefix="rerail-probe-bytecode-") as bytecode:
  sys.pycache_prefix=bytecode
  with contextlib.redirect_stdout(sys.stderr):result=observations()
 print(json.dumps(result))
