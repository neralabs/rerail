# levels.py: 0090's *levels, checked* over a program's derivations, for rec.py and ynm_reach.py (decision 0328).
# A derivation is (head, own): `own` the rows of the head's own recursive component it reads (other strata's rows
# are already checked into whether the derivation holds). A row's level is a witness: it has a derivation whose own
# rows are all of lower levels. The least level is the round of a derive the row is first made in.
import heapq
from collections import defaultdict

INF = float('inf')

def least(derivs, seed=None, targets=None):
    """Levels of `targets` (every head when None): rows of `seed` are present at their given levels; a target's
    level is one more than the highest of its first derivation complete in level order (0 with no own rows)."""
    users = defaultdict(list); pend = []; heap = []; L = {}
    for i, (h, own) in enumerate(derivs):
        if targets is not None and h not in targets:
            pend.append(None); continue
        need = set(own); pend.append(len(need))
        for o in need: users[o].append(i)
        if not need: heap.append((0, 0, h))
    for r, l in (seed or {}).items():
        if r in users: heap.append((l, 1, r))
    heapq.heapify(heap); done = set()
    while heap:
        l, kind, r = heapq.heappop(heap)
        if r in done: continue
        done.add(r)
        if kind == 0: L[r] = l
        for i in users.get(r, ()):
            pend[i] -= 1
            if pend[i] == 0:
                h = derivs[i][0]
                if h not in done: heapq.heappush(heap, (l + 1, 0, h))
    return L

def maintain(L, Ro, Rn, derivs_new, lost_heads, dependents):
    """An edit's removals and adds by levels. `derivs_new`: the derivations that hold after the edit (the new
    state's and the old ones still valid); `lost_heads`: heads of the old derivations that no longer hold;
    `dependents(x)`: heads of old derivations reading x. Candidates are checked in level order: one stays when a
    derivation reads only present rows of lower levels; one that does not goes, for now, and its dependents are
    candidates. Then the rows gone and the rows added are levelled from the present rows: a gone row so made comes
    back at a new level (a rewrite). Returns the new levels and the counts."""
    by_head = defaultdict(list)
    for h, own in derivs_new: by_head[h].append(own)
    heap = [(L.get(x, INF), x) for x in set(lost_heads) if x in Ro]
    heapq.heapify(heap)
    gone = set(); seen = set(); probes = 0
    while heap:
        l, x = heapq.heappop(heap)
        if x in gone: continue
        seen.add(x); probes += 1
        if any(all(o in Ro and o not in gone and L.get(o, INF) < l for o in own) for own in by_head[x]):
            continue
        gone.add(x)
        for u in dependents(x):
            if u in Ro and u not in gone: heapq.heappush(heap, (L.get(u, INF), u))
    present = Ro - gone
    added = Rn - Ro
    targets = gone | added
    made = least([(h, own) for h, own in derivs_new if h in targets], seed={r: L[r] for r in present if r in L},
                 targets=targets)
    kept = present | set(made)
    back = gone & set(made)
    NL = {r: L[r] for r in present if r in L}
    NL.update(made)
    return NL, dict(candidates=len(seen), probes=probes, gone=len(gone), back=len(back), added=len(added & set(made)),
                    errors=len(kept ^ Rn), unlevelled=sum(1 for r in present if r not in L), _seen=seen, _back=back)
