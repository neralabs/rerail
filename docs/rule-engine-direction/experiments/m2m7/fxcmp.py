import json
idx=json.load(open('fx/index.json')); nat={x['i']:x for x in json.load(open('fx/native.json'))}
agg={'n=m':0,'n!=m':0,'n=o':0,'m=o':0}
for e in idx:
  i=e['i']; m=set(json.load(open(f'fx/{i}.mine.json'))['keys'])
  n=set(nat[i]['keys']); o=set(json.dumps(json.loads(x),separators=(',',':')) for x in e['findings'])
  if m==n: agg['n=m']+=1
  else:
    agg['n!=m']+=1; print('DIFF',i,e['check'],e['test'],'native-only',sorted(n-m)[:3],'mine-only',sorted(m-n)[:3], nat[i]['failures'][:1])
  if n!=o: print('  n!=o',i,e['check'],e['test'],sorted(n-o)[:2],sorted(o-n)[:2],nat[i]['failures'][:1])
  agg['n=o']+= n==o; agg['m=o']+= m==o
print(agg, 'native keys',sum(len(x['keys']) for x in nat.values()),'oracle',sum(len(e['findings']) for e in idx))
