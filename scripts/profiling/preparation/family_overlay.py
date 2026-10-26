#!/usr/bin/env python3
"""Isolate terminal SQL fusion on the actual quality policy. Never edits shipped rules."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil


def resolve(value, options):
    if isinstance(value, str):
        return value
    if isinstance(value, dict) and '$option' in value:
        key = options[value['$option']]
        key = str(key).lower() if isinstance(key, bool) else str(key)
        return resolve(value.get('cases', {}).get(key, value.get('default')), options)
    raise ValueError('Unsupported SQL option expression')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('policies',type=Path)
    p.add_argument('output',type=Path)
    args=p.parse_args()
    if args.output.exists():
        raise ValueError('Use a new output directory')
    shutil.copytree(args.policies,args.output)
    path=args.output/'data-layer-serverplan-quality.json'
    source=path.read_bytes();rule=json.loads(source)
    options={k:v['default'] for k,v in rule['clauses'][0]['define']['options'].items()}
    clause=next(c for c in rule['clauses'] if c['id']=='quality-contracts-1')['relational']
    names=[f'diagnostics_{i}' for i in range(7)]
    terminal=[s for s in clause['stages'] if s['name'] in names]
    if [s['name'] for s in terminal]!=names or any(set(s)-{'name','query'} for s in terminal):
        raise ValueError('Terminal family shape changed; inspect before adapting')
    # Each terminal relation has exactly one consumer, the final UNION ALL. Its
    # original query remains intact. Other native, deferred, and recursive cuts stay.
    expected=' UNION ALL '.join(f'SELECT * FROM {name}' for name in names)
    if clause['query']!=expected:
        raise ValueError('Final consumer changed')
    # Keep the actual authored-query limit. Broad fusion belongs after logical
    # validation in the engine, not in a policy rewrite that raises its allowance.
    capabilities=Path(__file__).resolve().parents[3]/'native/src/code_rule_dsl/capabilities.json'
    byte_limit=json.loads(capabilities.read_text())['limits']['maxAstPatternBytes']
    selected=[];parts=[]
    for stage in terminal:
        part=f'"{stage["name"]}" AS NOT MATERIALIZED ({resolve(stage["query"],options)})'
        if sum(len(p.encode()) for p in parts)+len(part.encode())+len(expected.encode())+len(parts)+5<=byte_limit-1024:
            selected.append(stage['name']);parts.append(part)
    domains={name:set() for name in options}
    def discover(value):
        if isinstance(value,dict) and '$option' in value:
            domains[value['$option']].update(value.get('cases',{}))
            for branch in [*value.get('cases',{}).values(),value.get('default')]:discover(branch)
    for stage in terminal:discover(stage['query'])
    option_names=list(options)
    def expression(index,values):
        if index==len(option_names):
            definitions=','.join(f'"{s["name"]}" AS NOT MATERIALIZED ({resolve(s["query"],values)})' for s in terminal if s['name'] in selected)
            query='WITH '+definitions+' '+expected
            if len(query.encode())>byte_limit:raise ValueError('An option combination exceeds the existing query limit')
            return query
        name=option_names[index]
        def value(key):return key=='true' if isinstance(options[name],bool) else key
        fallback=options[name] if isinstance(options[name],bool) else '__rerail_unknown_option__'
        return {'$option':name,'cases':{key:expression(index+1,{**values,name:value(key)}) for key in sorted(domains[name])},
                'default':expression(index+1,{**values,name:fallback})}
    clause['query']=expression(0,{})
    clause['stages']=[s for s in clause['stages'] if s['name'] not in selected]
    path.write_text(json.dumps(rule,indent=2)+'\n')
    (args.output.parent/(args.output.name+'-manifest.json')).write_text(json.dumps({
        'format':'rerail.terminal-family-overlay/v1','sourceSha256':hashlib.sha256(source).hexdigest(),
        'overlaySha256':hashlib.sha256(path.read_bytes()).hexdigest(),
        'removedCuts':selected,'retainedCuts':[name for name in names if name not in selected],'preservedOptions':{k:sorted(v) for k,v in domains.items()},'authoredQueryByteLimit':byte_limit,
        'scope':'Actual terminal diagnostic SQL producers under the existing query-size limit. Authored options and defaults preserved. '
                'Same input domain, predicates, row multiplicity, deferred and native producer cuts. '
                'No shipped policy or engine alternative changes.'},indent=2)+'\n')


if __name__=='__main__':
    main()
