# ynm_reach.py DIR: yes/no/maybe on the spike's dense closure (maintain.rs, MAINTAIN_EDGES=DIR), edit by edit.
#   reach(a, b) :- edge(a, b).      reach(a, c) :- reach(a, b), edge(b, c).
# Removals first (the old edges less the removed), as rec.py: candidates are the heads of lost derivations; one probe
# each, head bound, over rows not candidates ("yes"), else open and its dependents become candidates; then every
# candidate settled by a least fixpoint from the rows that never were, in rounds while a "yes" falls. Checked
# against the closure derived again. Work: probes, and the derivations the settling fixpoint tries.
import glob,os,sys
from collections import defaultdict
D=sys.argv[1]
def edges(n): return {tuple(map(int,l.split())) for l in open(f'{D}/edge-{n:02d}.tsv') if l.strip()}
def closure(E):
    out=defaultdict(set);R=set()
    for a,b in E: out[a].add(b)
    for a in {a for a,_ in E}:
        seen=set();st=list(out[a])
        while st:
            b=st.pop()
            if b in seen: continue
            seen.add(b);st.extend(out.get(b,()))
        R|={(a,b) for b in seen}
    return R
n=len(glob.glob(f'{D}/edge-*.tsv'))
E=edges(0);R=closure(E)
print(f'state 00: {len(E)} edges, {len(R)} reach rows')
for k in range(1,n):
    E2=edges(k);gone=E-E2;M=E-gone           # M: the edges kept; removals are maintained against M
    out=defaultdict(set);inn=defaultdict(set)
    for a,b in M: out[a].add(b);inn[b].add(a)
    byfirst=defaultdict(set)
    for a,b in R: byfirst[a].add(b)
    def derivable(a,c,ok):                   # a derivation of reach(a,c) over M and rows ok() accepts
        if (a,c) in M: return True
        return any(ok((a,b)) for b in inn[c] if b in byfirst[a])
    C=set();cand=[]
    for a,c in gone:                          # rule 1's lost derivations, and rule 2's through the edge
        cand.append((a,c))
        for x in list(byfirst):
            if a in byfirst[x]: cand.append((x,c))
    cand0=list(cand)
    yes=set();opn=set();probes=0;rounds=0;tried=0
    while True:
        rounds+=1
        while cand:
            r=cand.pop()
            if r in C or r not in R: continue
            C.add(r);probes+=1
            if derivable(*r,lambda q: q in R and q not in C): yes.add(r);continue
            opn.add(r)
            a,c=r
            for d in out[c]: cand.append((a,d))  # its dependents: reach(a,d) :- reach(a,c), edge(c,d)
        # settle: every candidate from the rows that never were one, least fixpoint
        sure=R-C;back=set();changed=True
        while changed:
            changed=False
            for r in C-back:
                tried+=1
                if derivable(*r,lambda q: q in sure or q in back): back.add(r);changed=True
        fell=yes-back
        if not fell: break
        for r in fell:                        # a "yes" that fell: its dependents are candidates now
            yes.discard(r);opn.add(r)
            for d in out[r[1]]: cand.append((r[0],d))
    # Sound: a candidate stays only when proved from below (rec.py's `prove`): its cone (the rows its derivations
    # read, transitively) derived again from rule 1's rows; proved and refuted rows kept for the edit.
    RU=R
    proved=set();refuted=set()
    def prove(x):
        if x in proved: return True
        cone={x};st=[x]
        while st:
            a,c=st.pop()
            for b in inn[c]:
                o=(a,b)
                if o in RU and o not in proved and o not in refuted and o not in cone: cone.add(o);st.append(o)
        local=set();todo=list(cone);ch=True
        while ch:
            ch=False
            for r in cone-local:
                if derivable(*r,lambda q: q in proved or q in local): local.add(r);ch=True
        proved.update(local);refuted.update(cone-local)
        return x in local
    C2=set();work=list(cand0);gone2=set()
    while work:
        r=work.pop()
        if r in C2 or r not in R: continue
        C2.add(r)
        if prove(r): continue
        gone2.add(r)
        for d in out[r[1]]: work.append((r[0],d))
    sound_err=len(((R-gone2))^closure(M))
    kept=(R-C)|back
    want=closure(M)
    errors=len(kept^want)
    no=sum(1 for r in opn if not derivable(*r,lambda q: q in R and q!=r))
    removed=len(R-want)
    # DRed's overdelete, for comparison: everything a removed row's derivations reach
    over=set();st=[(a,c) for a,c in gone if (a,c) in R]
    for a,c in gone:
        for x in byfirst:
            if a in byfirst[x] and (x,c) in R: st.append((x,c))
    while st:
        r=st.pop()
        if r in over: continue
        over.add(r);st.extend((r[0],d) for d in out[r[1]] if (r[0],d) in R)
    print(f'edit {k:02d}: edges -{len(gone)} +{len(E2-E)}; reach rows removed {removed}; DRed overdeletes {len(over)}; '
          f'YNM candidates {len(C)} (yes at once {len(yes)}, open {len(opn)}: no {no}, maybe {len(opn)-no}), '
          f'settled back {len(back)-len(yes)}, rounds {rounds}, probes {probes}, settle tries {tried}; errors {errors}; '
          f'sound: candidates {len(C2)}, proved {len(proved)}, refuted {len(refuted)}, errors {sound_err}')
    E=E2;R=closure(E2)
