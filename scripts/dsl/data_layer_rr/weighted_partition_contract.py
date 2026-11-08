"""Compare one native demand batch with the pinned size-balanced partitioner."""
import argparse
import importlib
import json
from pathlib import Path
import random
import sys
import tempfile

from migration import PIN, ROOT, git, sha, write_json


def cases():
    result=[dict(id='empty',bins=4,files=[]),
            dict(id='zeros',bins=8,files=[('c.py',0),('a.py',0),('b.py',0)]),
            dict(id='path-components',bins=2,files=[('a.py',5),('a/x.py',5),('a-/x.py',5),('é.py',5),('β.py',5)])]
    for bins in [1,2,9]:
        result.append(dict(id=f'duplicates-{bins}',bins=bins,files=[('same.py',3),('a.py',2),('same.py',3),('b.py',1)]))
    rng=random.Random(73019)
    for count in [1,2,7,16,30,51,64,127]:
        for bins in [1,2,3,8,32]:
            files=[(f'p{rng.randrange(max(1,count//2))}/a{rng.randrange(5)}.py',rng.randrange(98)) for _ in range(count)]
            result.append(dict(id=f'random-{count}-{bins}',bins=bins,files=files))
    return result


CASES=cases()
FORMAT='rerail.weighted-partition-contract/v1'


def spec(value):
    return dict(value,files=[list(pair) for pair in value['files']])


def input_rows(value):
    final=dict(value['files'])
    return [dict(row=i,key='\0'.join(Path(path).parts),weight=final[path]) for i,(path,_) in enumerate(value['files'])]


def source_case(model,value):
    with tempfile.TemporaryDirectory(prefix='rerail-offline-weighted-') as temporary:
        root=Path(temporary)
        for path,size in value['files']:
            target=root/path;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(b'x'*size)
        paths=[root/path for path,_ in value['files']]
        occurrence={id(path):i for i,path in enumerate(paths)}
        chunks=model._partition_symbol_scan_files(paths,worker_count=value['bins'])
        expected=[[occurrence[id(path)] for path in chunk] for chunk in chunks]
        rows=[dict(row=i,key='\0'.join(path.relative_to(root).parts),weight=path.stat().st_size) for i,path in enumerate(paths)]
        if rows!=input_rows(value):raise ValueError('Physical fixture metadata differs from the declared input')
        return dict(spec=spec(value),rows=rows,expected=expected)


def program(records):
    text='''.policy "weighted-partition-contract" revision 1
.language python
.mode log
.scope dirs "__no_primary__"
.decl work(task: number, row: number, key: symbol, weight: number)
.decl limits(task: number, bins: number)
'''
    for task,record in enumerate(records):
        text+=f'limits({task}, {record["spec"]["bins"]}).\n'
        for row in record['rows']:
            text+=f'work({task}, {row["row"]}, {json.dumps(row["key"],ensure_ascii=False)}, {row["weight"]}).\n'
    return text+'''
.violation Assignment(at: span, task: number, row: number, bin: number, ordinal: number) code "BALANCED_ROW" "assignment"
Assignment(@place("",0), task, row, bin, ordinal) :- balanced_rows(task,row,bin,ordinal,work,limits).
'''


def native_results(records,diagnostics):
    tasks={i:[] for i in range(len(records))}
    for row in diagnostics:
        if row['code']!='BALANCED_ROW':raise ValueError(f'Unexpected native diagnostic: {row}')
        p=row['parameters'];tasks[p['task']].append((p['ordinal'],p['bin'],p['row']))
    result=[]
    for task,record in enumerate(records):
        rows=sorted(tasks[task])
        if [i for i,_,_ in rows]!=list(range(len(record['rows']))):raise ValueError('Missing or repeated occurrence ordinals')
        if sorted(r for _,_,r in rows)!=list(range(len(record['rows']))):raise ValueError('Missing or repeated input occurrences')
        bins=[bin for _,bin,_ in rows]
        if any(not 0<=bin<min(record['spec']['bins'],len(record['rows'])) for bin in bins):
            raise ValueError('Partition bin outside the allocated range')
        if bins!=sorted(bins):raise ValueError('Assembly ordinals must follow bin order')
        chunks=[];last=None
        for _,bin,row in rows:
            if bin!=last:chunks.append([]);last=bin
            chunks[-1].append(row)
        result.append(dict(id=record['spec']['id'],expected=record['expected'],actual=chunks,passed=chunks==record['expected']))
    return result


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('command',choices=['source','native'])
    for name in ['source','engine','baseline','output']:p.add_argument('--'+name,type=Path,required=name=='output')
    args=p.parse_args();result=dict(format=FORMAT,sourcePin=PIN,cases=[],complete=False,passed=False)
    if args.command=='source':
        if git(args.source,'rev-parse','HEAD')!=PIN:p.error('Pinned source required')
        sys.path[:0]=[str(args.source),str(args.source/'scripts')]
        model=importlib.import_module('lint_lib.symbol_graph')
        result['sourceSha256']=sha((args.source/'scripts/lint_lib/symbol_graph.py').read_bytes())
        result['cases']=[source_case(model,value) for value in CASES]
    else:
        from replay import execute_native
        baseline=json.loads(args.baseline.read_text())
        if baseline['format']!=FORMAT or baseline['sourcePin']!=PIN or not baseline['complete'] or not baseline['passed']:p.error('Complete pinned recording required')
        if [r['spec'] for r in baseline['cases']]!=[spec(v) for v in CASES] or [r['rows'] for r in baseline['cases']]!=[input_rows(v) for v in CASES]:p.error('Complete unchanged inputs required')
        path=ROOT/'.rerail/rr-migration-20261106/internal-dead-code/weighted/contract.rr';path.parent.mkdir(parents=True,exist_ok=True);path.write_text(program(baseline['cases']))
        native=execute_native(args.engine,path,[],{})
        result.update(cases=native_results(baseline['cases'],native['diagnostics']),native=native,
                      engineSha256=sha(args.engine.read_bytes()),baselineSha256=sha(args.baseline.read_bytes()))
    result.update(complete=True,passed=all(c.get('passed',True) for c in result['cases']))
    write_json(args.output,result)
    print(json.dumps(dict(cases=len(result['cases']),passed=result['passed'])))
    return 0 if result['passed'] else 1


if __name__=='__main__':raise SystemExit(main())
