"""Normalise the 113 whole-file regex clauses (112 forbid.regex + strategy-file-size under $option) into
out/text_clauses.json: id, pattern (resolved for default options), lines opts, content gates, scope globs, source."""
import json, glob, os, re
RULES = '/Users/jeremy/orca/workspaces/rerail/Pulling-it-all-together/packs/data-layer/candidates/rules'
DEFAULTS = {'entryPoint': 'all', 'oracleAnalyzer': False, 'sourceGate': True}
def resolve(v):
    if isinstance(v, dict) and '$option' in v:
        opt = DEFAULTS.get(v['$option'])
        if 'cases' not in v: return v.get('default', opt)
        key = str(opt).lower() if isinstance(opt, bool) else opt
        return resolve(v['cases'].get(key, v.get('default')))
    if isinstance(v, dict): return {k: resolve(x) for k, x in v.items()}
    if isinstance(v, list): return [resolve(x) for x in v]
    return v
out = []
for f in sorted(glob.glob(os.path.join(RULES, '*.json'))):
    d = json.load(open(f))
    for c in d['clauses']:
        fb = c.get('forbid')
        if fb is None: continue
        fb = resolve(fb)
        if not (isinstance(fb, dict) and 'regex' in fb): continue
        r = fb['regex']
        lines = r.get('lines')
        out.append(dict(rule=d['id'], clause=c['id'], source=r['source'], pattern=r['pattern'],
            lines=lines, content=(r.get('content') or {}), match_include=r.get('include', []), match_exclude=r.get('exclude', []),
            scope_include=d['scope'].get('include', []), scope_exclude=d['scope'].get('exclude', []),
            paths_include=(r.get('paths') or {}).get('include'), paths_exclude=(r.get('paths') or {}).get('exclude', [])))
json.dump(out, open('out/text_clauses.json', 'w'), indent=1)
print(len(out), sum(o['source'] == 'file.content' for o in out))
