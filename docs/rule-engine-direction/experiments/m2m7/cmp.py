import json,sys,collections
nat=json.load(open(sys.argv[1]))['rules']; mine=set(json.load(open(sys.argv[2]))['keys'])
n=set(k for v in nat.values() for k in v['keys'])
print('native',len(n),'mine',len(mine),'both',len(n&mine))
fp=sorted(mine-n); fn=sorted(n-mine)
print('extra(mine only)',len(fp),collections.Counter(json.loads(x)[3] for x in fp)); print(fp[:10])
print('missing(native only)',len(fn),collections.Counter(json.loads(x)[3] for x in fn)); print(fn[:10])
