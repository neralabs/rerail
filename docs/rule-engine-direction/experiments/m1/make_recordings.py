import json, sys, hashlib
src = sys.argv[1]; out = sys.argv[2]
rules = ['serverplan_state_tripwires','serverplan_steps','serverplan_quality','internal_dead_code','public_api_triples','async_sync_delegation','transaction_patterns','async_sync_boundary']
d = json.load(open(src))
print('commit', d['commit'], 'files', len(d['files']), 'events', len(d['events']), 'mismatch', len(d['source_mismatches']))
checks = [e['check'] for e in d['events']]
for r in rules:
    ev = [e for e in d['events'] if e['check'] == r]
    assert len(ev) == 1, (r, checks)
    sub = dict(d); sub['events'] = ev
    json.dump(sub, open(f'{out}/{r}.json', 'w'))
    print(r, 'expected', len(ev[0]['findings']), 'index', checks.index(r))
