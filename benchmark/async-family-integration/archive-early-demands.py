#!/usr/bin/env python3
"""Archive reports and SQL profiles, without corpus databases or executables."""
import argparse,gzip,hashlib,json,subprocess
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('root',type=Path);p.add_argument('out',type=Path);a=p.parse_args()
a.out.mkdir(parents=True,exist_ok=True)
selected=[]
for name in ['control','production','timings','edges','positive-profile']:
    base=a.root/name
    for f in sorted(base.rglob('*')):
        if f.is_file() and f.suffix in ['.json','.log','.sql','.tsv','.txt']:selected.append(f)
for name in ['positive.json','gates.json','merged-build-compatibility.patch','merge-build.log','merge-build-fixed.log','early-build.log','authored-tests.log','projection-tests.log','composition-tests.log','eligibility-test.log','registration-test.log']:
    selected.append(a.root/name)
files=[]
for f in selected:
    data=f.read_bytes();rel=f.relative_to(a.root);dest=a.out/(str(rel)+'.gz');dest.parent.mkdir(parents=True,exist_ok=True)
    dest.write_bytes(gzip.compress(data,compresslevel=9,mtime=0))
    files.append({'path':str(rel),'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()})
binaries={name:hashlib.sha256((a.root/name).read_bytes()).hexdigest() for name in ['merged-engine','early-engine']}
sources={}
for base in ['native/src/code_rule_dsl','docs/rule-syntax']:
    for f in sorted(Path(base).rglob('*')):
        if f.is_file() and f.suffix in ['.rs','.rr']:sources[str(f)]=hashlib.sha256(f.read_bytes()).hexdigest()
manifest={'schema':'rerail-early-demands-v1','integrationMerge':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),
    'baseline':'integrationMerge plus merged-build-compatibility.patch; frozen rule-syntax tree from integrationMerge',
    'corpus':'5,725 Python files; corpus-fce4dc72; see candidate-and-index-spikes.md',
    'root':str(a.root),'binarySha256':binaries,'sourceSha256':sources,'files':files}
(a.out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(f'Archived {len(files)} evidence files; {sum(f.stat().st_size for f in a.out.rglob("*") if f.is_file())} bytes')
