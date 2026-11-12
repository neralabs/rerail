#!/usr/bin/env python3
"""Offline statistical-planning experiment over immutable native-lab captures.

Preparation and exhaustive oracle execution are reported separately. Production
planning is unchanged. The adapter requires existing indexes with count/max-group
metadata; it never builds an index while probing.
"""
import argparse
import array
import hashlib
import itertools
import json
import math
import random
import sqlite3
import statistics
import sys
import time
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path


class Exhausted(Exception):
    pass


class Budget:
    def __init__(self, memory=2 << 20, probes=512, work=50_000):
        self.memory_limit, self.probe_limit, self.work_limit = memory, probes, work
        self.memory = self.peak = self.probes = self.work = 0
        self.scratch = 0

    def reserve(self, size):
        if self.memory + size > self.memory_limit:
            return False
        self.memory += size
        self.peak = max(self.peak, self.memory)
        return True

    def charge(self):
        if self.work >= self.work_limit:
            raise Exhausted()
        self.work += 1

    def draw(self):
        if self.probes >= self.probe_limit:
            raise Exhausted()
        self.probes += 1


def constant(term):
    return int(term[2:]) if term.startswith('i:') else None


def variables(args):
    return {x for x in args if x.startswith('v')}


@dataclass(frozen=True)
class Atom:
    source: str
    args: tuple

    def literals(self):
        return tuple((c, constant(t)) for c, t in enumerate(self.args) if constant(t) is not None)


class Source:
    """Common prepared facts/indexes: not statistical storage or probe preparation."""
    def __init__(self, rows, width):
        self.rows, self.width = tuple(rows), width
        rows = self.rows
        self.keys = set(rows)
        self.indexes = [defaultdict(list) for _ in range(width)]
        for i, row in enumerate(rows):
            for c, value in enumerate(row):
                self.indexes[c][value].append(i)
        self.maximum = [max(map(len, ix.values()), default=0) for ix in self.indexes]
        self.revision = hashlib.sha256(repr(rows).encode()).hexdigest()

    def group(self, atom, binding):
        eq = []
        for c, t in enumerate(atom.args):
            v = constant(t)
            if v is not None:
                eq.append((c, v))
            elif t in binding:
                eq.append((c, binding[t]))
        if len(eq) == self.width:
            row = tuple(v for _, v in sorted(eq))
            return (row,) if row in self.keys else ()
        if not eq:
            return self.rows
        ids = min((self.indexes[c].get(v, ()) for c, v in eq), key=len)
        # Return a lazy view; do not allocate a potentially enormous result vector.
        return (self.rows[i] for i in ids)

    def seed_ids(self, atom):
        eq = atom.literals()
        if not eq:
            return range(len(self.rows))
        return min((self.indexes[c].get(v, ()) for c, v in eq), key=len)

    def upper(self, atom, bound):
        if all(constant(t) is not None or t in bound for t in atom.args):
            return 1 if self.rows else 0
        bounds = [len(self.rows)]
        for c, t in enumerate(atom.args):
            if constant(t) is not None:
                bounds.append(len(self.indexes[c].get(constant(t), ())))
            elif t in bound:
                bounds.append(self.maximum[c])
        return min(bounds)


def bind(atom, row, binding):
    out = dict(binding)
    for t, v in zip(atom.args, row):
        expected = constant(t)
        if expected is not None:
            if v != expected:
                return None
        elif t in out:
            if out[t] != v:
                return None
        else:
            out[t] = v
    return out


@dataclass
class Case:
    name: str
    sources: dict
    atoms: list
    guards: list
    head: tuple
    expected: set | None = None

    def orders(self):
        for order in itertools.permutations(range(len(self.atoms))):
            bound = set()
            for i in order:
                vs = variables(self.atoms[i].args)
                if bound and not bound & vs:
                    break
                bound |= vs
            else:
                yield order

    def walk(self, order, charge, output=None, seed_row=None):
        def run(pos, binding, remaining):
            if pos == len(order):
                if output is not None:
                    output.add(tuple(constant(t) if constant(t) is not None else binding[t] for t in self.head))
                return
            atom = self.atoms[order[pos]]
            charge()  # indexed lookup, including an unsuccessful probe
            rows = (seed_row,) if pos == 0 and seed_row is not None else self.sources[atom.source].group(atom, binding)
            for row in rows:
                charge()  # physical candidate row examined, not only emitted rows
                merged = bind(atom, row, binding)
                if merged is None:
                    continue
                pending = []
                ok = True
                for a, b in remaining:
                    if all(constant(t) is not None or t in merged for t in (a, b)):
                        charge()
                        av = constant(a) if constant(a) is not None else merged[a]
                        bv = constant(b) if constant(b) is not None else merged[b]
                        if not av < bv:
                            ok = False
                            break
                    else:
                        pending.append((a, b))
                if ok:
                    run(pos + 1, merged, pending)
        run(0, {}, self.guards)

    def truth(self, order, limit=5_000_000):
        work = 0
        def charge():
            nonlocal work
            if work >= limit:
                raise Exhausted('offline oracle ceiling reached')
            work += 1
        out = set()
        self.walk(order, charge, out)
        if self.expected is not None:
            assert out == self.expected, (self.name, order, 'capture parity')
        return work, out

    def bound(self, order):
        """Proven per-seed work upper bound from existing posting maxima/keys."""
        bound, sizes = set(), []
        for i in order[1:]:
            bound |= variables(self.atoms[order[len(sizes)]].args)
            sizes.append(self.sources[self.atoms[i].source].upper(self.atoms[i], bound))
        result = 0
        for n in reversed(sizes):
            result = 1 + n * (1 + len(self.guards) + result)
        # walk counts the first lookup too; keeping it here is conservative.
        return 2 + len(self.guards) + result


class Summary:
    def __init__(self, source, atoms, rng, budget):
        width = source.width
        # Conservative accounting includes Python containers, pattern keys, and counters.
        self.counts = {}
        self.groups = {}
        self.count_checks = 0
        self.sample = array.array('q')
        self.n = len(source.rows)
        self.width = width
        self.revision = source.revision
        if not budget.reserve(1024):
            raise Exhausted("statistics memory")
        for atom in atoms:
            key = atom.literals()
            columns=tuple(c for c,_ in key)
            extra=256 if columns not in self.groups else 0
            if (key and key not in self.counts and (columns in self.groups or len(self.groups)<8)
                    and budget.reserve(512 + 128 * len(key) + extra)):
                self.counts[key] = 0
                self.groups.setdefault(columns,{})[tuple(v for _,v in key)]=key
        capacity = min(256, max(0, max(0, budget.memory_limit - budget.memory - 128) // max(8 * width, 1)))
        if budget.reserve(128 + capacity * width * 8):
            self.sample = array.array('q', [0]) * (capacity * width)
        else:
            capacity = 0
        self.capacity = capacity
        # One streaming pass; this CPU/row cost is explicitly measured by the harness.
        for i, row in enumerate(source.rows):
            for columns, wanted in self.groups.items():
                self.count_checks += 1
                key=wanted.get(tuple(row[c] for c in columns))
                if key is not None:
                    self.counts[key] += 1
            j = i if i < capacity else rng.randrange(i + 1) if capacity else capacity
            if j < capacity:
                for c, value in enumerate(row):
                    self.sample[j * width + c] = value
        self.length = min(capacity, self.n)

    def rows(self):
        for i in range(getattr(self, 'length', 0)):
            yield tuple(self.sample[i * self.width:(i + 1) * self.width])

    def actual_bytes(self):
        # Source rows/indexes are borrowed and excluded; retained sample/counters included.
        return (sys.getsizeof(self) + sys.getsizeof(self.__dict__) + sys.getsizeof(self.sample)
                + sys.getsizeof(self.counts) + sys.getsizeof(self.groups)
                + sum(sys.getsizeof(mask)+sys.getsizeof(group)+sum(sys.getsizeof(values) for values in group) for mask,group in self.groups.items())
                + sum(sys.getsizeof(k) + sum(sys.getsizeof(p) + sum(map(sys.getsizeof, p)) for p in k)
                      + sys.getsizeof(v) for k, v in self.counts.items()))


def estimate(case, order, summaries, correlated):
    rows, work, domains = 1.0, 0.0, {}
    for i in order:
        a = case.atoms[i]
        source, summary = case.sources[a.source], summaries[a.source]
        fixed = a.literals()
        n = len(source.rows)
        if fixed:
            if summary is not None and fixed in summary.counts:
                n = summary.counts[fixed]
            else:
                for c, _ in fixed:
                    n /= max(len(source.indexes[c]), 1)
        sample = list(summary.rows()) if summary is not None else []
        subset = [r for r in sample if all(r[c] == v for c, v in fixed)] if correlated else sample
        out = float(n)
        for c, t in enumerate(a.args):
            if constant(t) is not None:
                continue
            # Existing global NDV is common prepared metadata in this adapter.
            d = len(source.indexes[c])
            if correlated and subset:
                counts = Counter(r[c] for r in subset)
                # Deliberately test the existing extrapolation heuristic, not an oracle NDV.
                d = min(d, max(1, round(len(counts) * n / len(subset))))
            if t in domains:
                out /= max(d, domains[t], 1)
            else:
                domains[t] = min(n, d)
        work += rows * (1 + out)
        rows *= out
    return work


class Probe:
    def __init__(self, case, order, budget):
        self.case, self.order, self.budget = case, order, budget
        first = case.atoms[order[0]]
        self.source = case.sources[first.source]
        self.ids = self.source.seed_ids(first)
        self.nseed = len(self.ids)
        self.n = self.total = 0
        self.censored = False
        self.maximum = case.bound(order)
        self.full = self.nseed <= 32
        self.reserved = budget.reserve(1024 + len(order) * 64)

    def extend(self, target, rng):
        if not self.reserved or self.censored:
            return
        while self.n < min(target, self.nseed if self.full else target):
            before = self.budget.work
            try:
                self.budget.draw()
                j = self.n if self.full else rng.randrange(self.nseed)
                row = self.source.rows[self.ids[j]]
                self.case.walk(self.order, self.budget.charge, seed_row=row)
            except Exhausted:
                self.censored = True
                break
            cost = self.budget.work - before - 1
            assert cost <= self.maximum, (cost, self.maximum)
            self.n += 1
            self.total += cost

    def interval(self, delta):
        if self.censored:
            return (0.0, math.inf)
        if self.nseed == 0:
            return (1.0, 1.0)
        if not self.n:
            return (0.0, math.inf)
        mean = self.total / self.n
        # Exact enumeration of a small demand set is preferable to random probing.
        error = 0 if self.full and self.n == self.nseed else self.maximum * math.sqrt(math.log(2 / delta) / (2 * self.n))
        lo = 1 + self.nseed * max(0, mean - error)
        hi = 1 + self.nseed * (mean + error)
        return lo, hi


def collect_summaries(case, rng, budget):
    out = {}
    for name, source in case.sources.items():
        try:
            out[name] = Summary(source, [a for a in case.atoms if a.source == name], rng, budget)
        except Exhausted:
            out[name] = None
    return out


def select(case, summaries, rng, budget, queries, known_current=None, cache=None):
    for name, summary in summaries.items():
        if summary is not None and summary.revision != case.sources[name].revision:
            raise ValueError("statistics belong to another relation revision")
    revision = tuple((n, source.revision) for n, source in sorted(case.sources.items()))
    if known_current is not None and known_current[0] != revision:
        raise ValueError("observed work belongs to another relation revision")
    if len(case.atoms)>4 or max(s.width for s in case.sources.values())>8:
        raise ValueError("spike supports at most four atoms and eight columns")
    needed_scratch=4096+256*(max(s.width for s in case.sources.values())*80+512)
    if needed_scratch>budget.scratch:
        if not budget.reserve(needed_scratch-budget.scratch):
            return tuple(range(len(case.atoms))), 'no planning scratch budget', {}
        budget.scratch=needed_scratch
    orders = list(case.orders())
    baseline = tuple(range(len(case.atoms)))
    scores = {o: estimate(case, o, summaries, True) for o in orders}
    finalists = list(dict.fromkeys([baseline] + sorted(orders, key=scores.get)[:2]))
    if all(o == baseline or scores[o] >= .95 * scores[baseline] for o in finalists):
        return baseline, 'no promising challenger', {}
    before_work = budget.work
    probes = {}
    for o in finalists:
        key = (tuple((case.sources[a.source].revision,a.args) for a in case.atoms),tuple(case.guards),o)
        if cache is not None and key in cache:
            probes[o] = cache[key]
        else:
            probe = Probe(case,o,budget)
            probes[o] = probe
            if cache is not None and probe.reserved:
                cache[key] = probe
    # Union bound over candidates, stages, and all queries sharing this job budget.
    delta = .05 / (max(queries, 1) * len(finalists) * 6)
    chosen, reason = baseline, 'uncertain: retain current plan'
    for target in [16, 32, 64, 128, 256, 512]:
        for o, probe in probes.items():
            if o != baseline or known_current is None:
                probe.extend(target, rng)
        intervals = {o: p.interval(delta) for o, p in probes.items()}
        if known_current is not None:
            intervals[baseline]=(known_current[1],known_current[1])
        best = min(intervals, key=lambda o: intervals[o][1])
        if best != baseline and intervals[best][1] + (budget.work - before_work) < .95 * intervals[baseline][0]:
            chosen, reason = best, 'improvement bounds cover probe work'
            break
        if budget.probes >= budget.probe_limit or budget.work >= budget.work_limit:
            break
    return chosen, reason, {str(o): {'n': p.n, 'censored': p.censored, 'bounds': list(intervals[o])} for o, p in probes.items()}


def captures(directory):
    groups = defaultdict(list)
    for f in directory.glob('prog_*.sqlite'):
        groups[f.stem.rsplit('-', 1)[0]].append(f)
    chosen = []
    for files in groups.values():
        files.sort(key=lambda f: int(f.stem.rsplit('-', 1)[1]))
        chosen.extend(dict.fromkeys([files[0], files[-1]]))
    result = []
    for f in chosen:
        db = sqlite3.connect(f.as_uri() + '?mode=ro', uri=True)
        spec = db.execute("SELECT value FROM meta WHERE key='spec'").fetchone()[0]
        seeds = [l.split() for l in spec.splitlines() if l.startswith('seed ')]
        if len(seeds) != 1 or any(l.startswith('join ') for l in spec.splitlines()):
            raise ValueError(f'unsupported recursive/multi-seed capture: {f}')
        lines = bytes.fromhex(seeds[0][2]).decode().splitlines()
        atoms, guards, head, sources = [], [], None, {}
        for line in lines:
            op, *args = line.split()
            if op == 'pos':
                parts = args[0].split(':')
                if parts[:1] != ['t']:
                    raise ValueError(line)
                name = bytes.fromhex(parts[2]).decode()
                cols = bytes.fromhex(parts[3]).decode()
                if name not in sources:
                    rows = list(db.execute(f'SELECT {cols} FROM "{name}"'))
                    sources[name] = Source(rows, len(args) - 1)
                atoms.append(Atom(name, tuple(args[1:])))
            elif op == 'lt':
                guards.append(tuple(args))
            elif op == 'head':
                head = tuple(args)
            else:
                raise ValueError(f'unsupported capture operation {op}')
        expected = set(db.execute(f'SELECT * FROM out{seeds[0][1]}'))
        result.append(Case(f.name, sources, atoms, guards, head, expected))
        db.close()
    return result


def synthetic():
    out = []
    for name, skew in [('uniform', False), ('rare-heavy-tail', True), ('correlated', True)]:
        a = [(i, i % 200) for i in range(800)]
        b = [(i % 200, i % 23, int((i % 200) < 4) if skew else i % 2) for i in range(4000)]
        c = [(i,) for i in range(23) if i % 3 == 0]
        if name == 'rare-heavy-tail':
            b += [(199, 1000 + j, 1) for j in range(600)]
        # Relations have set semantics.
        src = {'a': Source(a, 2), 'b': Source(list(dict.fromkeys(b)), 3), 'c': Source(c, 1)}
        out.append(Case(name, src, [Atom('a', ('v0','v1')), Atom('b', ('v1','v2','i:1')), Atom('c', ('v2',))], [], ('v0','v2')))
    return out


def shared_summaries(cases, rng, budget):
    needs = defaultdict(list)
    sources = {}
    for case in cases:
        for name, source in case.sources.items():
            key=(source.revision,source.width)
            sources[key]=source
            needs[key].extend(a for a in case.atoms if a.source==name)
    registry={}
    for key, source in sources.items():
        try: registry[key]=Summary(source,needs[key],rng,budget)
        except Exhausted: registry[key]=None
    return [{name:registry[(source.revision,source.width)] for name,source in case.sources.items()} for case in cases],registry


def run(cases, seeds):
    truth_started = time.process_time_ns()
    truth = []
    for case in cases:
        costs = {}
        expected = None
        for order in case.orders():
            cost, rows = case.truth(order)
            if expected is None:
                expected = rows
            assert rows == expected
            costs[order] = cost
        truth.append(costs)
    truth_cpu = time.process_time_ns() - truth_started
    trials = []
    for seed, mode in itertools.product(range(seeds), ['cold','observed-current']):
        rng, budget = random.Random(seed), Budget()
        budget.reserve(4096)  # one shared RNG, not one generator per relation
        started = time.process_time_ns()
        summaries, registry = shared_summaries(cases,rng,budget)
        ingest_cpu = time.process_time_ns() - started
        actual_bytes = sys.getsizeof(rng) + sum(s.actual_bytes() for s in registry.values() if s is not None)
        assert actual_bytes <= budget.memory <= budget.memory_limit
        choices = []
        cache = {}
        started = time.process_time_ns()
        for case, summary, costs in zip(cases, summaries, truth):
            orders = list(costs)
            original = tuple(range(len(case.atoms)))
            marginal = min(orders, key=lambda o: estimate(case,o,summary,False))
            correlated = min(orders, key=lambda o: estimate(case,o,summary,True))
            revision=tuple((n,source.revision) for n,source in sorted(case.sources.items()))
            evidence=(revision,costs[original]) if mode=='observed-current' else None
            chosen, reason, intervals = select(case,summary,rng,budget,len(cases),evidence,cache)
            oracle = min(costs.values())
            choices.append({'case':case.name,'kind':'captured' if case.expected is not None else 'synthetic','currentWork':costs[original],'oracleWork':oracle,
                'marginalWork':costs[marginal],'correlatedWork':costs[correlated], 'probedWork':costs[chosen],
                'reason':reason,'order':chosen,'intervals':intervals,'regret':costs[chosen]/oracle})
        cache_bytes=sys.getsizeof(cache)
        for key,probe in cache.items():
            cache_bytes += (sys.getsizeof(key)+sys.getsizeof(key[0])+sum(sys.getsizeof(x) for x in key[0])
                +sys.getsizeof(key[1])+sys.getsizeof(key[2])+sys.getsizeof(probe)+sys.getsizeof(probe.__dict__)
                +sum(sys.getsizeof(getattr(probe,n)) for n in ['nseed','n','total','maximum','full','censored']))
        assert actual_bytes+cache_bytes+budget.scratch <= budget.peak <= budget.memory_limit
        trials.append({'seed':seed,'mode':mode,'summaryCount':len(registry),'counterLookups':sum(s.count_checks for s in registry.values() if s is not None),'ingestedRows':sum(s.n for s in registry.values() if s is not None),'probeCacheEntries':len(cache),'ingestCpuNs':ingest_cpu,'planningCpuNs':time.process_time_ns()-started,
            'statisticsBytes':actual_bytes,'probeCacheBytes':cache_bytes,'accountedPeakBytes':budget.peak,'probes':budget.probes,'probeWork':budget.work,'scratchBytes':budget.scratch,'choices':choices})
    summary = {}
    for mode,kind in itertools.product(['cold','observed-current'],['synthetic','captured']):
        group=[c for t in trials if t['mode']==mode for c in t['choices'] if c['kind']==kind]
        if not group: continue
        summary[mode+'/'+kind]={}
        for field in ['currentWork','marginalWork','correlatedWork','probedWork']:
            values=[c[field]/c['oracleWork'] for c in group]
            summary[mode+'/'+kind][field]={'medianRegret':statistics.median(values),'maxRegret':max(values),
                'over2x':sum(v>2 for v in values),'totalWork':sum(c[field] for c in group),
                'worseThanCurrent':sum(c[field]>c['currentWork'] for c in group)}
    return {'schema':1,'oracleCpuNs':truth_cpu,'seedCount':seeds,'caseCount':len(cases),
        'budget':{'memoryBytes':2<<20,'seedProbes':512,'probeWork':50000},'summary':summary,'trials':trials}


def historical_controls(cases):
    out=[]
    for c in cases:
        order=(2,1,0) if 'rr_embed_req' in c.name else (1,2,3,0)
        out.append(Case('historical-order/'+c.name,c.sources,[c.atoms[i] for i in order],c.guards,c.head,c.expected))
    return out


def main():
    p=argparse.ArgumentParser();p.add_argument('--capture',type=Path);p.add_argument('--seeds',type=int,default=32);p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    if args.seeds<1:p.error('seeds must be positive')
    start=time.process_time_ns();groups={'synthetic':synthetic()}
    if args.capture:
        real=captures(args.capture.resolve())
        groups['current-captures']=real
        groups['historical-controls']=historical_controls(real)
    preparation=time.process_time_ns()-start
    result={'schema':2,'prepareCpuNs':preparation,'jobs':{name:run(cases,args.seeds) for name,cases in groups.items()},
        'inputs':{name:[{'case':c.name,'relations':{n:{'rows':len(s.rows),'revision':s.revision} for n,s in c.sources.items()}} for c in cases] for name,cases in groups.items()}}
    def finite(value):
        if isinstance(value,float) and not math.isfinite(value):return None
        if isinstance(value,dict):return {k:finite(v) for k,v in value.items()}
        if isinstance(value,(list,tuple)):return [finite(v) for v in value]
        return value
    args.output.write_text(json.dumps(finite(result),indent=2,allow_nan=False))
    print(json.dumps({name:d['summary'] for name,d in result['jobs'].items()},indent=2))

if __name__=='__main__': main()
