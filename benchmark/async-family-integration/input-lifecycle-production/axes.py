import pathlib
source=pathlib.Path(__file__).with_name('run.py').read_text();exec(source[:source.index("checks=['facts::core::'")])
corpus=pathlib.Path('/Users/jeremy/orca/artifacts/async-family-comparison/corpus-fce4dc72');subset=root/'axes-corpus';subset.mkdir(exist_ok=True)
chosen=sorted(corpus.rglob('*.py'),key=lambda p:(-p.stat().st_size,str(p)))[:100]
for i,p in enumerate(chosen):
 dest=subset/f'f{i:03}.py'
 if not dest.exists():dest.symlink_to(p)
(root/'axes-files.json').write_text(json.dumps([str(p) for p in chosen],indent=2)+'\n')
rows=[]
for way in ['baseline','tape']:
 for arm in ['baseline','production']:
  name=f'axes-{way}-{arm}';e=base.copy();e.update(RERAIL_MEASURE_CORPUS=str(subset),RERAIL_MEASURE=way,RERAIL_PROFILE=str(root/name/'profile'))
  m=invoke(arm,name,'logic::plan::tests::measure_inside_three_ways',e,True);m.update(arm=arm,way=way)
  rows.append(m);(root/'axes-results.json').write_text(json.dumps(rows,indent=2)+'\n')
  print(name,m['processCpu'],m['processWall'],flush=True)
