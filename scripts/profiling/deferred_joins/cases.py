"""Inputs and reader schedules for the projected join wide(a,b) = exists k: l(a,k), r(b,k)."""

import hashlib
import struct

CASES = (
    "cartesian",
    "empty",
    "sparse",
    "hot-key",
    "mixed-readers",
    "full",
    "discovered",
    "rounds-edits",
)
MODES = ("eager", "direct", "demand", "factors", "reduced", "tabled")


def factors(case):
    # One hot key produces a Cartesian product. The other keys supply duplicate
    # witnesses, partitions, and keys belonging to only one side.
    n = 160
    if case == "cartesian":
        return {(a, 0) for a in range(n)}, {(b, 0) for b in range(n)}
    left = {(a, a % 8 + 1) for a in range(n)}
    right = {(b, b % 8 + 1) for b in range(n)}
    left.update((a, 0) for a in range(n if case == "hot-key" else n // 2))
    right.update((b, 0) for b in range(n))
    left.add((n, 41))
    right.add((n, 42))
    return left, right


def schedule(case, db=None):
    sparse = [(i, (i * 19 + 3) % 161) for i in range(0, 161, 3)]
    if case == "empty":
        return [("cold", [], [])]
    if case in ("sparse", "cartesian"):
        return [("cold", sparse, [])]
    if case == "hot-key":
        return [("cold", [(0, None), (None, 0)] + sparse, [])]
    if case == "mixed-readers":
        return [("cold", sparse + [(7, None), (None, 9), (160, None)], [])]
    if case == "full":
        return [("cold", [(None, None)], [])]
    if case == "discovered":
        return [("seed", [(0, None)], []), ("follow-up", [], []), ("repeat", [], [])]
    if case == "rounds-edits":
        first = [(0, 0), (0, 999), (160, None)]
        more = first + [(1, 0), (None, 0)]
        return [
            ("cold", first, []),
            ("new-keys", more, []),
            ("warm", more, []),
            ("drop-one-witness", more, [("delete", 0, 0)]),
            ("drop-last-witness", more, [("delete", 0, 1)]),
            ("insert-after-miss", more, [("insert", 999, 1)]),
        ]
    if case == "posthog":
        keys = list(db.execute("SELECT a,b FROM requests ORDER BY a,b"))
        return [("cold", keys, []), ("warm", keys, [])]
    if case == "coverage":
        # Artificial broader readers over the captured workload. Keep their labels
        # separate from the policy's actual correlated requests.
        keys = list(db.execute("SELECT DISTINCT a,NULL FROM requests ORDER BY a"))
        return [
            ("left-bound", keys, []),
            ("repeat", keys, []),
            ("full", [(None, None)], []),
        ]
    raise ValueError(case)


def reader_keys(case, phase, keys, previous):
    if case == "discovered" and phase == "follow-up":
        return [(b, 0) for _, b in sorted(previous[(0, None)])]
    if case == "discovered" and phase == "repeat":
        return list(previous)
    return keys


def digest(answers, keys):
    result = hashlib.sha256()
    count = 0
    for i, key in enumerate(keys):
        rows = sorted(answers.get(key, ()))
        result.update(struct.pack(">QQ", i, len(rows)))
        for a, b in rows:
            result.update(struct.pack(">qq", a, b))
        count += len(rows)
    return {"rows": count, "sha256": result.hexdigest()}


def oracle(case, db=None):
    if case == "posthog":
        output = set(db.execute("SELECT a,b FROM expected"))
        phases = schedule(case, db)
        if not output.issubset(phases[0][1]):
            raise ValueError("native answers fall outside the captured demand")
        return {
            phase: digest({pair: [pair] for pair in output}, keys)
            for phase, keys, _ in phases
        }
    if case == "coverage":
        # These wider readers have no native output. Use the full relational join as their oracle.
        result = {}
        wide = db.execute("SELECT DISTINCT l.a,r.b FROM r INNER JOIN l USING(k)")
        rows = list(wide)
        for label, keys, _ in schedule(case, db):
            by_key = {key: [] for key in keys}
            for a, b in rows:
                for key in ((a, None), (None, None)):
                    if key in by_key:
                        by_key[key].append((a, b))
            result[label] = digest(by_key, keys)
        return result
    left, right = factors(case)
    checks = {}
    answers = {}
    for phase, keys, edits in schedule(case):
        keys = reader_keys(case, phase, keys, answers)
        for op, b, k in edits:
            if op == "delete":
                right.remove((b, k))
            else:
                right.add((b, k))
        wide = {(a, b) for a, k in left for b, j in right if k == j}
        answers = {
            key: [
                (a, b)
                for a, b in wide
                if (key[0] is None or key[0] == a) and (key[1] is None or key[1] == b)
            ]
            for key in keys
        }
        checks[phase] = digest(answers, keys)
    return checks
