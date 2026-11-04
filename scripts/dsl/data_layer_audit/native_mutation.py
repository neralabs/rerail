#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Create always-clean diagnostic policies while preserving real fact producers."""
import argparse
import json
from pathlib import Path


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('packs/data-layer/candidates'))
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    for folder in ('rules','facts'):
        target=args.output/folder;target.mkdir(parents=True,exist_ok=True)
        paths=list((args.source/folder).glob('*.json'))
        if folder=='rules' and args.source.name=='candidates':paths+=list((args.source.parent/'rules').glob('*.json'))
        for path in paths:
            policy=json.loads(path.read_text())
            policy['clauses']=[clause for clause in policy['clauses'] if 'define' in clause]+[
              dict(id='always-clean',forbid=dict(regex=dict(source='file.content',pattern=r'\b\B')),
                violation=dict(code='CONTROL',message='Always-clean negative control',anchor='witness.span'))]
            (target/path.name).write_text(json.dumps(policy,indent=2)+'\n')


if __name__=='__main__':main()
