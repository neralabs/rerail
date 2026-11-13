import json,pathlib,statistics
root=pathlib.Path(__file__).resolve().parent
variants=[r['variant']+'-'+r['label'] for r in json.loads((root/'runs.json').read_text()) if r['profile']];summary={}
for v in variants:
 p=root/(v+'.json')
 if not p.exists():continue
 d=json.loads(p.read_text()); spans=d['profile']['spans'];costs={}
 for name in ['tape restore','facts from tape','node columns','child CSR','kind postings','class postings','projected','identity maps','projection lines','token spans','projection metadata']:
  costs[name]=sum(s['ownCpuSeconds'] for s in spans if name in s['name'].split('/'))
 payload={}
 for dec in d['profile']['decisions']:
  if dec['topic']=='input payload':
   for key,n in json.loads(dec['text']).items():payload[key]=payload.get(key,0)+n*dec['count']
 summary[v]={'cpu':d['cpuNs']/1e9,'wall':d['wallNs']/1e9,'phases':d['report']['phases'],'costs':costs,'payload':payload,'statements':sum(x[4] for x in d['report']['phases']),'vm':sum(s['runs']*0+s['vmSteps'] for s in d['profile']['statements']),'keys':d['keyCount']}
(root/'profile-summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary,indent=2))
