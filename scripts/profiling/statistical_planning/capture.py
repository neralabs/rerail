#!/usr/bin/env python3
"""Capture the selected current-planner strata from the O125 qualification replay."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--environment',type=Path,help='Existing O125 replay environment JSON')
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    binary=args.binary.resolve(strict=True)
    out=args.output.resolve();out.mkdir(parents=True,exist_ok=False)
    env=dict(os.environ)
    if args.environment:env.update(json.loads(args.environment.read_text()))
    env.pop('RDL_EXPLAIN',None)
    env.update(RERAIL_RELATIONAL_LAB=str(out/'lab'),
        RERAIL_RELATIONAL_LAB_ONLY='rr_embed_req,type_open__sup6,type_open__sup11',
        RERAIL_PROFILE=str(out/'profile'),O125_SAVE=str(out/'rows'),
        RERAIL_TAPE_CACHE=str(out/'facts.sqlite'),RAYON_NUM_THREADS='4')
    with (out/'run.log').open('w') as log:
        result=subprocess.run(['nice','-n','19',str(binary),'spike_o125_replay',
            '--ignored','--nocapture','--test-threads=1'],env=env,stdout=log,stderr=subprocess.STDOUT)
    (out/'provenance.json').write_text(json.dumps({'binarySha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
        'environmentSha256':hashlib.sha256(args.environment.read_bytes()).hexdigest() if args.environment else None,
        'exit':result.returncode},indent=2))
    result.check_returncode()

if __name__=='__main__':main()
