"""Snapshot source-recorded contract groups with the common parity validator."""
import argparse
import gzip
import json
from pathlib import Path
import scanner_checkpoint
from migration import ROOT,sha
from validate_supplied_facts_checkpoint import require


def validate(value):
    require(value['format']=='rerail.recorded-contract-progress/v1','Wrong contract progress format')
    require(value['sourceFormat']==value['source']['format'],'Wrong contract source format')
    legacy=dict(value,format='rerail.scanner-checkpoint/v1',scanners=value['reports'])
    summary=scanner_checkpoint.validate(legacy)
    summary['recordedCasesPerAPI']=summary.pop('scannerCasesPerAPI')
    summary['sourceFormat']=value['sourceFormat']
    return summary


def pack(work,check):
    value=scanner_checkpoint.pack(work,check)
    value.update(format='rerail.recorded-contract-progress/v1',sourceFormat=value['source']['format'],reports=value.pop('scanners'),
        remaining=['Other rule contracts, persistent updates, behavior-removal controls, and final rule admission remain separate requirements.'])
    path=Path(__file__);text=path.read_text();value['harness'][path.relative_to(ROOT).as_posix()]=dict(text=text,sha256=sha(text.encode()))
    value['summary']=validate(value);return value


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',choices=['pack','validate']);parser.add_argument('path',type=Path);parser.add_argument('--work',type=Path);parser.add_argument('--check')
    args=parser.parse_args()
    if args.command=='pack':
        value=pack(args.work,args.check);args.path.write_bytes(gzip.compress(json.dumps(value,ensure_ascii=False,separators=(',',':')).encode(),mtime=0))
    else:
        with gzip.open(args.path,'rt') as stream:value=json.load(stream)
    print(json.dumps(validate(value),sort_keys=True))
