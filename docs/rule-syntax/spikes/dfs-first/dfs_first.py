"""dfs_first: ordered first-match search, set-based until the end (spike).

Two implementations of the same contract:

- `oracle`: a line-for-line port of the Data-layer branch's ordered_first.rs `Graph::first`
  (native/src/code_rule_dsl/relational/ordered_first.rs), the reproduction of the Python
  scripts' memoised depth-first search. No supplied memo or visiting state.
- `SetBased`: SQLite does the set work (live, next, first_of, chain entry points); Tarjan
  finds the knot; Python searches only where a root's next chain enters the knot.

Contract (both):
  graph   edges (src, dst, ord, loc), sinks {id: loc}, roots in order
  mode    "node": a root that is a sink is its own hit; "edge": only edge targets count
  share   False: a fresh memo per root (dfs_first);
          True: one memo shared across roots in root order (dfs_first_memo, the scripts)
  result  {root: (sink, loc) or None}. loc is the location of the root's first edge on
          the path, or the sink's own location when the root is the hit (node mode).
Edges leave a node in (ord, dst) order, as ordered_first.rs's ORDER BY.
"""
import sqlite3
from collections import defaultdict


# ---------------------------------------------------------------- oracle ----
class Oracle:
    def __init__(self, edges, sinks):
        self.out = defaultdict(list)
        for s, d, o, loc in sorted(edges, key=lambda e: (e[0], e[2], e[1])):
            self.out[s].append((d, loc))
        self.sinks = dict(sinks)
        self.steps = 0

    def first(self, root, on_edge, memo):
        self.steps += 1
        if root in memo:
            return memo[root]
        if not on_edge and root in self.sinks:
            memo[root] = (root, self.sinks[root])
            return memo[root]
        stack = [[root, 0, None]]                        # node, next edge, via location
        visiting = {root}
        while True:
            self.steps += 1
            fr = stack[-1]
            es = self.out[fr[0]]
            if fr[1] >= len(es):
                stack.pop()
                visiting.discard(fr[0])
                memo[fr[0]] = None                       # the poisoned entry, inside cycles
                if not stack:
                    return None
                continue
            child, loc = es[fr[1]]
            fr[1] += 1
            if on_edge and child in self.sinks:
                found = (child, loc)
            elif child in visiting:
                continue
            elif child in memo:
                c = memo[child]
                found = None if c is None else (c[0], loc)
            elif not on_edge and child in self.sinks:
                memo[child] = (child, self.sinks[child])
                found = (child, loc)
            else:
                visiting.add(child)
                stack.append([child, 0, loc])
                continue
            if found is not None:
                sink, loc = found
                while stack:
                    f = stack.pop()
                    visiting.discard(f[0])
                    memo[f[0]] = (sink, loc)
                    if f[2] is not None:
                        loc = f[2]
                return (sink, loc)


def oracle(edges, sinks, roots, mode, share):
    g = Oracle(edges, sinks)
    memo, out = {}, {}
    for r in roots:
        out[r] = g.first(r, mode == "edge", memo if share else {})
    return out, g.steps


# ------------------------------------------------------------- set-based ----
SCHEMA = """
CREATE TABLE edge(src INTEGER, dst INTEGER, ord INTEGER, loc INTEGER);
CREATE TABLE sink(id INTEGER PRIMARY KEY, loc INTEGER);
CREATE TABLE root(id INTEGER PRIMARY KEY, ord INTEGER);
"""

# 1. live: nodes that can reach a sink. One backward closure.
LIVE = """
CREATE TABLE live AS
WITH RECURSIVE l(v) AS (SELECT id FROM sink UNION SELECT e.src FROM edge e JOIN l ON e.dst = l.v)
SELECT v FROM l;
CREATE UNIQUE INDEX live_v ON live(v);
"""

# 2. next: each node's first edge, in (ord, dst) order, into a live node. One argmin per node.
NEXT = """
CREATE TABLE good AS
SELECT e.src, e.dst, e.ord, e.loc,
       row_number() OVER (PARTITION BY e.src ORDER BY e.ord, e.dst) AS k
FROM edge e JOIN live l ON l.v = e.dst
WHERE e.src <> e.dst OR e.dst IN (SELECT id FROM sink);   -- the search skips a call to itself
CREATE TABLE nxt AS SELECT src, dst, loc FROM good WHERE k = 1;
CREATE UNIQUE INDEX nxt_src ON nxt(src);
CREATE INDEX nxt_dst ON nxt(dst);
"""

# 3. first_of: follow next to a sink. next is a function, so this is a walk up an in-forest;
#    nodes on a next-cycle get no row.
FIRST = """
CREATE TABLE first_of AS
WITH RECURSIVE f(v, hit) AS (
  SELECT n.src, n.dst FROM nxt n JOIN sink s ON s.id = n.dst
  UNION ALL
  SELECT n.src, f.hit FROM nxt n JOIN f ON n.dst = f.v
  WHERE NOT EXISTS (SELECT 1 FROM sink s WHERE s.id = n.dst))
SELECT v, hit FROM f;
CREATE UNIQUE INDEX first_v ON first_of(v);
"""

# 4. knot: live non-sink nodes on a cycle of live non-sink nodes (a strongly connected
#    component with two or more nodes). Only these can be cut by the visiting set or poisoned
#    in the memo. Components come from Tarjan's algorithm, a linear native pass over g2 that
#    yields a relation; see knot().
G2 = """
CREATE TABLE g2 AS
SELECT DISTINCT e.src, e.dst FROM edge e
JOIN live a ON a.v = e.src JOIN live b ON b.v = e.dst
WHERE e.src NOT IN (SELECT id FROM sink) AND e.dst NOT IN (SELECT id FROM sink)
  AND e.src <> e.dst;                 -- a self-call cannot cut its own search
CREATE TABLE knot(v INTEGER PRIMARY KEY);
"""


def knot(pairs):
    """Nodes of strongly connected components with two or more nodes (iterative Tarjan)."""
    out = defaultdict(list)
    for s, d in pairs:
        out[s].append(d)
    index, low, on, stack, res, n = {}, {}, set(), [], [], 0
    for v0 in list(out):
        if v0 in index:
            continue
        work = [(v0, 0)]
        while work:
            v, i = work.pop()
            if i == 0:
                index[v] = low[v] = n
                n += 1
                stack.append(v)
                on.add(v)
            es = out.get(v, ())
            if i < len(es):
                work.append((v, i + 1))
                w = es[i]
                if w not in index:
                    work.append((w, 0))
                elif w in on:
                    low[v] = min(low[v], index[w])
                continue
            if work:
                u = work[-1][0]
                low[u] = min(low[u], low[v])
            if low[v] == index[v]:
                comp = []
                while True:
                    w = stack.pop()
                    on.discard(w)
                    comp.append(w)
                    if w == v:
                        break
                if len(comp) > 1:
                    res.extend(comp)
    return res


# 5. ent: the first knot node on each node's next chain. A node with none is "pure": its
#    set-based answer is exact whatever ran before. Every next-cycle lies in the knot.
ENT = """
CREATE TABLE ent AS
WITH RECURSIVE t(v, u) AS (
  SELECT v, v FROM knot
  UNION ALL
  SELECT n.src, t.u FROM nxt n JOIN t ON n.dst = t.v
  WHERE n.src NOT IN (SELECT v FROM knot) AND n.src NOT IN (SELECT id FROM sink))
SELECT v, u FROM t;
CREATE UNIQUE INDEX ent_v ON ent(v);
"""


class SetBased:
    def __init__(self, edges, sinks, path=":memory:"):
        self.db = sqlite3.connect(path)
        self.db.executescript(SCHEMA)
        self.db.executemany("INSERT INTO edge VALUES (?,?,?,?)", edges)
        self.db.executemany("INSERT INTO sink VALUES (?,?)", sinks.items())
        self.db.executescript("CREATE INDEX edge_dst ON edge(dst); CREATE INDEX edge_src ON edge(src, ord, dst);")
        self.db.executescript(LIVE + NEXT + FIRST + G2)
        self.db.executemany("INSERT INTO knot VALUES (?)",
                            [(v,) for v in knot(self.db.execute("SELECT src, dst FROM g2"))])
        self.db.executescript(ENT)
        q = self.db.execute
        self.sinks = dict(sinks)
        self.live = {v for (v,) in q("SELECT v FROM live")}
        self.knot = {v for (v,) in q("SELECT v FROM knot")}
        self.ent = dict(q("SELECT v, u FROM ent"))
        self.nxt = {s: (d, loc) for s, d, loc in q("SELECT src, dst, loc FROM nxt")}
        self.first = dict(q("SELECT v, hit FROM first_of"))
        # Adjacency only where the search can run: nodes whose chain meets the knot, and sinks
        # (edge mode searches from a sink root).
        self.out = defaultdict(list)
        for s, d, loc in q("""SELECT e.src, e.dst, e.loc FROM edge e
                              WHERE e.src IN (SELECT v FROM ent) OR e.src IN (SELECT id FROM sink)
                              ORDER BY e.src, e.ord, e.dst"""):
            self.out[s].append((d, loc))
        self.steps = 0
        self.searches = 0

    def _search(self, root, on_edge, memo):
        """ordered_first.rs's search. It steps into a node only if that node's chain meets the
        knot; at a pure node it takes the set-based answer."""
        self.searches += 1
        self.steps += 1
        if root in memo:
            return memo[root]
        stack, visiting = [[root, 0, None]], {root}
        while True:
            self.steps += 1
            fr = stack[-1]
            es = self.out[fr[0]]
            if fr[1] >= len(es):
                stack.pop()
                visiting.discard(fr[0])
                memo[fr[0]] = None
                if not stack:
                    return None
                continue
            child, loc = es[fr[1]]
            fr[1] += 1
            if on_edge and child in self.sinks:
                found = (child, loc)
            elif child not in self.live or child in visiting:
                continue                                  # dead subtrees find nothing
            elif child in memo:
                c = memo[child]
                found = None if c is None else (c[0], loc)
            elif child in self.sinks:
                found = (child, loc)                      # node mode: a sink child is the hit
            elif child not in self.ent:
                found = (self.first[child], loc)          # pure: exact without searching
            else:
                visiting.add(child)
                stack.append([child, 0, loc])
                continue
            if found is not None:
                sink, loc = found
                while stack:
                    f = stack.pop()
                    visiting.discard(f[0])
                    memo[f[0]] = (sink, loc)
                    if f[2] is not None:
                        loc = f[2]
                return (sink, loc)

    def run(self, roots, mode, share):
        on_edge = mode == "edge"
        memo, cache, out = {}, {}, {}
        for r in roots:
            if r in self.sinks:
                out[r] = self._search(r, on_edge, memo if share else {}) if on_edge else (r, self.sinks[r])
            elif r not in self.live:
                out[r] = None
            elif r not in self.ent:
                out[r] = (self.first[r], self.nxt[r][1])          # pure
            else:
                u = self.ent[r]                                    # the chain is exact up to u
                if share:
                    a = self._search(u, on_edge, memo)
                    if a is None:                                  # u was poisoned: back up past it
                        out[r] = self._search(r, on_edge, memo)
                        continue
                else:
                    if u not in cache:                             # without a shared memo, u's
                        cache[u] = self._search(u, on_edge, {})    # answer is the same for every root
                    a = cache[u]
                out[r] = a if u == r else (a[0], self.nxt[r][1])
        return out
