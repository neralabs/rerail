import pathlib
source=pathlib.Path(__file__).with_name('run.py').read_text()
exec(source[:source.index("\nfor variant in ['baseline','combined']:")])
records=json.loads((root/'runs.json').read_text())
for variant in ['baseline','combined','baseline']:
 corpus(variant,'warm-profile'+str(len(records)),True)
for rep in range(3):
 for variant in ['lines','postings','reserve','combined']:
  order=['baseline',variant] if rep%2==0 else [variant,'baseline']
  for arm in order:corpus(arm,'pair-'+variant+'-'+str(rep))
pairs={}
for v in ['lines','postings','reserve','combined']:
 rows=[]
 for rep in range(3):
  pair=[r for r in records if r['label']=='pair-'+v+'-'+str(rep)]
  b=next(r for r in pair if r['variant']=='baseline');s=next(r for r in pair if r['variant']==v)
  rows.append(dict(baselineCpu=b['cpu'],spikeCpu=s['cpu'],cpuSaved=b['cpu']-s['cpu'],baselineWall=b['wall'],spikeWall=s['wall'],wallSaved=b['wall']-s['wall']))
 pairs[v]={'pairs':rows,'medianCpuSaved':statistics.median(r['cpuSaved'] for r in rows),'medianWallSaved':statistics.median(r['wallSaved'] for r in rows)}
(root/'paired-summary.json').write_text(json.dumps(pairs,indent=2)+'\n');print(json.dumps(pairs,indent=2),flush=True)
