# rec.py [steps]: recursion in calls.rr's real edits, from the engine's derivations (drv_i) at each replay step.
# Reads $REC_DIR (default tmp/rec): meta.json (prov.py) and rel-NN/ (dump.sh). Decision 0328's measurements.
# Per state: rows of recursive relations, and those on a cycle of the derivation graph. Per edit: what recheck alone
# (no overdelete, no state) keeps wrongly; DRed's overdelete; and yes/no/maybe from the deltas, checked exact.
import json,glob,os,sys,time
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
import levels as LV
from collections import defaultdict
B=os.environ.get('REC_DIR','tmp/rec')
meta=json.load(open(os.path.join(B,'meta.json')))
comps=meta['comps']; drvs=meta['drv']
own_of={r:i for i,c in enumerate(comps) for r in c}
# A component is partitioned by its first column when every rule's own atoms read the head's first argument there
# (the spike's `partition_col`): derived again, only the keys an edit reaches.
def _parted(c):
    ds=[d for d in drvs if d['comp']==c]
    return bool(ds) and all(d['hargs'] and d['hargs'][0] is not None and all(l['args'] and l['args'][0]==d['hargs'][0]
               for l in d['lits'] if not l['neg'] and l['rel'] in comps[c]) for d in ds)
parted={c for c in range(len(comps)) if _parted(c)}
CARRY={}
# REC_CARRY=FILE: the carried levels from an earlier run of the steps before (a run under 2 minutes takes a few edits).
import pickle
if os.environ.get('REC_CARRY') and os.path.exists(os.environ['REC_CARRY']):
    CARRY.update(pickle.load(open(os.environ['REC_CARRY'],'rb')))
cache={}
def load(s,r):
    # a relation's rows at step s: every run's dump of it (the chunks' and the cross part's), as one set
    k=(s,r)
    if k not in cache:
        ps=glob.glob(f'{B}/rel-{s}/run*/{r}.tsv')
        # A stratum the engine did not run again (its inputs unchanged since the store's last run, the previous step)
        # writes no dump: its rows are the last step's that did. An empty relation writes an empty file.
        at=steps.index(s) if s in steps else 0
        while not ps and at>0:
            at-=1;ps=glob.glob(f'{B}/rel-{steps[at]}/run*/{r}.tsv')
        if not ps: cache[k]=None
        else: cache[k]=set(tuple(int(v) if v.lstrip('-').isdigit() else v for v in l.rstrip('\n').split('\t')) for p in ps for l in open(p) if l.strip())
    return cache[k]
def derivs(s):
    """[(comp, head, own_body_rows, nonown_pos, neg_checks)]"""
    out=[]
    for d in drvs:
        rows=load(s,d['drv']) or set()
        for b in rows:
            g=lambda cs: tuple(b[c[1]] for c in cs)
            head=(d['head'],g(d['hargs']))
            own=[];pos=[];neg=[]
            for l in d['lits']:
                if l['neg']:
                    neg.append((l['rel'],tuple(None if a is None else b[a[1]] for a in l['args'])))
                elif l['rel'] in comps[d['comp']]:
                    own.append((l['rel'],g(l['args'])))
                else:
                    pos.append((l['rel'],g(l['args'])))
            out.append((d['comp'],head,tuple(own),tuple(pos),tuple(neg)))
    return out
def facts(s,r):
    # another stratum's relation at step s: the cross part's copy (cp_r) and every run's own dump of r, as one set
    k=(s,'facts',r)
    if k not in cache:
        a=load(s,'cp_'+r); b=load(s,r)
        cache[k]=None if a is None and b is None else (a or set())|(b or set())
    return cache[k]
negix={}
def neg_hit(s,r,key):
    k=(s,r,tuple(i for i,v in enumerate(key) if v is not None))
    if k not in negix:
        ix=set()
        for row in facts(s,r) or ():
            ix.add(tuple(row[i] for i in k[2]))
        negix[k]=ix
    return tuple(v for v in key if v is not None) in negix[k]
def valid_in(s,d):
    _,_,_,pos,neg=d
    for r,row in pos:
        rows=facts(s,r)
        if rows is None: raise SystemExit(f'no dump of {r} at {s}')
        if row not in rows: return False
    for r,key in neg:
        if facts(s,r) is None: raise SystemExit(f'no dump of {r} at {s}')
        if neg_hit(s,r,key): return False
    return True
def rows_of(s):
    R=set()
    for c in comps:
        for r in c:
            for row in facts(s,r) or (): R.add((r,row))
    return R
def sccs(nodes,succ):
    """iterative tarjan; returns set of nodes in nontrivial SCCs"""
    idx={};low={};on=set();st=[];cyc=set();n=0
    for v0 in nodes:
        if v0 in idx: continue
        work=[(v0,iter(succ.get(v0,())))];idx[v0]=low[v0]=n;n+=1;st.append(v0);on.add(v0)
        while work:
            v,it=work[-1]
            adv=False
            for w in it:
                if w not in idx:
                    idx[w]=low[w]=n;n+=1;st.append(w);on.add(w);work.append((w,iter(succ.get(w,()))));adv=True;break
                elif w in on: low[v]=min(low[v],idx[w])
            if adv: continue
            work.pop()
            if work: low[work[-1][0]]=min(low[work[-1][0]],low[v])
            if low[v]==idx[v]:
                comp=[]
                while True:
                    w=st.pop();on.discard(w);comp.append(w)
                    if w==v: break
                if len(comp)>1 or v in succ.get(v,()): cyc.update(comp)
    return cyc
steps=sys.argv[1:] or ['00','01','02','03','04','05','06','07','08']
for s in steps:
    if not glob.glob(f'{B}/rel-{s}/run*'): raise SystemExit(f'no dumps for step {s} in {B} (did the run fail?)')
D={};R={}
def evict(keep):
    for k in [k for k in cache if k[0] not in keep]: del cache[k]
    for k in [k for k in negix if k[0] not in keep]: del negix[k]
    for k in [k for k in list(D) if k[:2] not in keep]: del D[k]
    for k in [k for k in list(R) if k not in keep]: del R[k]
def state(s):
    D[s]=derivs(s); dumped=rows_of(s)
    heads={d[1] for d in D[s]}
    # the relations as the program means them: every derivation's head (the drv rules run in the cross part, over
    # whole relations). The engine's dump is the same, except for `found`, which it derives by demand
    # (`.pragma "magic-transform"`), and a file-local relation the per-file memo kept: only the edited files' rows.
    # One store for every step: a store per step interns symbols and nodes apart, and rows do not compare.
    # REC_LAX=1: a dumped row with no derivation is reported, not refused (a chunked run's file-local dump; the
    # rows measured are the heads either way).
    if os.environ.get('REC_LAX') and not dumped<=heads:
        print(f'  warning {s}: {len(dumped-heads)} dumped rows with no derivation in {sorted({r for r,_ in dumped-heads})}')
    else:
        assert dumped<=heads, (s,len(dumped-heads),sorted({r for r,_ in dumped-heads}),sorted(dumped-heads)[:3])
    R[s]=heads
    if os.environ.get('REC_SELFCHECK'):
        bad=[d for d in D[s] if not valid_in(s,d)]
        assert not bad, (s,len(bad),bad[:2])
    succ=defaultdict(set)
    for d in D[s]:
        for o in d[2]: succ[o].add(d[1])
    cyc=sccs(list(R[s]),succ)
    byc=defaultdict(int)
    for x in cyc: byc[own_of[x[0]]]+=1
    print(f'state {s}: {len(R[s])} rows of recursive relations, {len(D[s])} derivations; in cycles: {len(cyc)}',
          {'+'.join(comps[c][:2]):n for c,n in byc.items()})
    D[s+'cyc']=cyc
def name(c): return '+'.join(comps[c][:3])
for a,b in zip(steps,steps[1:]):
    import time; t0=time.time()
    evict({a,b})
    if a not in R: state(a)
    if b not in R: state(b)
    print(f'  states loaded {time.time()-t0:.0f} s', flush=True)
    Ro,Rn=R[a],R[b]
    removed=Ro-Rn; added=Rn-Ro
    # mixed derivations: the new state's, and the old ones whose other-stratum literals still hold
    # an old derivation also in the new state holds there; only the others are checked against the new facts
    newset=set(D[b])
    old_valid=[];lost=[]
    for d in D[a]:
        (old_valid if d in newset or valid_in(b,d) else lost).append(d)
    M=D[b]+old_valid
    # GFP over old+new rows: recheck with a cascade, no overdelete
    S=set(Ro|Rn)
    by_head=defaultdict(list)
    for d in M: by_head[d[1]].append(d)
    users=defaultdict(set)
    for d in M:
        for o in d[2]: users[o].add(d[1])
    work=list(S)
    while work:
        x=work.pop()
        if x not in S: continue
        if not any(all(o in S for o in d[2]) for d in by_head[x]):
            S.discard(x); work.extend(u for u in users[x] if u in S)
    wrong=S-Rn
    assert Rn<=S
    # DRed: overdelete from the old derivations lost, along the old graph
    oldusers=defaultdict(set)
    for d in D[a]:
        for o in d[2]: oldusers[o].add(d[1])
    dele=set(d[1] for d in lost); work=list(dele)
    while work:
        x=work.pop()
        for u in oldusers[x]:
            if u not in dele: dele.add(u); work.append(u)
    # yes/no/maybe after one probe of each overdeleted row: yes = a derivation (new context) on rows outside the
    # overdeleted set; no = no derivation on any old or new row; maybe = only through other overdeleted rows
    RU=Ro|Rn
    yes=no=maybe=0; maybe_true=0
    for x in dele:
        ds=by_head[x]
        if any(all(o not in dele and o in RU for o in d[2]) for d in ds): yes+=1
        elif not any(all(o in RU for o in d[2]) for d in ds): no+=1
        else:
            maybe+=1
            if x in Rn: maybe_true+=1
    # yes/no/maybe, from the deltas alone. Candidates: the old rows that lost a derivation. A candidate with a
    # derivation (new context) whose own rows are all outside the candidates is kept at once ("yes"), and its
    # dependents are not touched. One that is not passes its old dependents on as candidates. Then the candidates
    # still open are settled together, from the kept rows up (a least fixpoint within them): "maybe" are those with
    # a derivation through other open candidates, "no" those with none.
    C=set(d[1] for d in lost if d[1] in Ro); YES=set(); work=list(C); probes=0
    while work:
        x=work.pop()
        if x in YES: continue
        probes+=1
        if any(all(o in RU and (o not in C or o in YES) for o in d[2]) for d in by_head[x]):
            YES.add(x); continue
        for u in oldusers[x]:
            if u in Ro and u not in C: C.add(u); work.append(u)
    opn=C-YES
    ynm_maybe=sum(1 for x in opn if any(all(o in RU for o in d[2]) for d in by_head[x]))
    ynm_no=len(opn)-ynm_maybe
    # Settle: every candidate (the "yes" ones too) from the rows outside the candidates up, a least fixpoint. A
    # candidate that does not come back is gone, and its dependents become candidates; again until none is added.
    rounds=0
    while True:
        rounds+=1
        settled=set(); changed=True
        while changed:
            changed=False
            for x in C-settled:
                if any(all(o in RU and (o not in C or o in settled) for o in d[2]) for d in by_head[x]):
                    settled.add(x); changed=True
        gone=C-settled
        more={u for x in gone for u in oldusers[x] if u in Ro and u not in C}
        if not more: break
        C|=more
    ynm_err=len(gone ^ removed)
    # Sound yes/no/maybe (Backward/Forward's backward phase): a candidate stays only when proved, bottom-up, from
    # derivations with no row of its own component: the rows its derivations read, transitively (its cone), derived
    # again from below. One probe trusts a row that is not a candidate, which a cycle can hold up by itself
    # (ynm_reach.py's ring). Proved rows are kept for the rest of the edit; the cone stops at them.
    proved=set(); refuted=set()
    def prove(x):
        if x in proved: return True
        cone={x}; st=[x]
        while st:
            r=st.pop()
            for d in by_head[r]:
                for o in d[2]:
                    if o in RU and o not in proved and o not in refuted and o not in cone: cone.add(o); st.append(o)
        local=set(); changed=True
        while changed:
            changed=False
            for r in cone-local:
                if any(all(o in RU and (o in proved or o in local) for o in d[2]) for d in by_head[r]):
                    local.add(r); changed=True
        proved.update(local); refuted.update(cone-local)
        return x in local
    C2=set(d[1] for d in lost if d[1] in Ro); work=list(C2); gone2=set()
    while work:
        x=work.pop()
        if prove(x): continue
        gone2.add(x)
        for u in oldusers[x]:
            if u in Ro and u not in C2: C2.add(u); work.append(u)
    sound_err=len(gone2 ^ removed)
    # Levels (0090, checked): minimal levels from the old state (fresh), and levels carried from the last edit.
    old_d=[(d[1],d[2]) for d in D[a]]; new_d=[(d[1],d[2]) for d in M]; lost_h=[d[1] for d in lost]
    dep=lambda x: oldusers[x]
    LF=LV.least(old_d)
    _,lf=LV.maintain(LF,Ro,Rn,new_d,lost_h,dep)
    if not CARRY: CARRY.update(LF)
    NL,lc=LV.maintain(CARRY,Ro,Rn,new_d,lost_h,dep)
    drift=sum(CARRY.get(r,0)-LF.get(r,0) for r in Ro if r in LF)/max(1,len(Ro))
    CARRY.clear(); CARRY.update(NL)
    # Partitions: the keys an edit reaches in partitioned components, and the rows derived again for them.
    keys={(own_of[x[0]],x[1][0]) for x in set(lost_h)|(Rn-Ro)|(Ro-Rn) if own_of[x[0]] in parted and x[1]}
    prows=sum(1 for x in Rn if x[1] and (own_of[x[0]],x[1][0]) in keys)
    lev_in_parted=sum(1 for x in lf['_seen'] if own_of[x[0]] in parted)
    ynm_back=len(opn & settled)
    rc=defaultdict(int)
    for x in removed: rc[own_of[x[0]]]+=1
    wc=defaultdict(int)
    for x in wrong: wc[own_of[x[0]]]+=1
    print(f'edit {a}->{b}: removed {len(removed)} added {len(added)}; lost derivations {len(lost)}; '
          f'recheck-only keeps wrongly {len(wrong)} {dict(__import__('collections').Counter(r for r,_ in wrong))}; '
          f'DRed overdeletes {len(dele)} (true {len(dele-Rn)}); one probe: yes {yes} no {no} maybe {maybe} (of which kept {maybe_true}); '
          f'removed in old cycles {len(removed & D[a+"cyc"])}; YNM: candidates {len(C)} (probes {probes}), yes at once {len(YES)}, open {len(opn)} (maybe {ynm_maybe}, no {ynm_no}), settled back {ynm_back}, final candidates {len(C)} in {rounds} rounds, errors {ynm_err}; on old cycles: candidates {len(C & D[a+"cyc"])}, kept {len(C & D[a+"cyc"] & settled)} (where one probe can keep a row wrongly, as on ynm_reach.py ring); sound: candidates {len(C2)}, proved {len(proved)}, refuted {len(refuted)}, errors {sound_err}; levels fresh: candidates {lf['candidates']}, gone {lf['gone']}, rewrites {lf['back']}, added {lf['added']}, errors {lf['errors']}; levels carried: candidates {lc['candidates']}, gone {lc['gone']}, rewrites {lc['back']}, errors {lc['errors']}, mean drift {drift:.2f}; partitions: {len(parted)} of {len(comps)} components, keys {len(keys)}, rows {prows} (level candidates there {lev_in_parted}); removed by comp {dict((name(c),n) for c,n in rc.items())}')
if os.environ.get('REC_CARRY'): pickle.dump(CARRY,open(os.environ['REC_CARRY'],'wb'))
