"""Four physical choices for the same logical join. All reads use SQLite's indexes."""

from collections import OrderedDict


def demanded_pair(db, mask):
    # Adorn each factor with the bound column. Keep the original tuple demand
    # in the final semijoin so that independently projected keys never cross.
    return f"""WITH
        lhs AS MATERIALIZED (SELECT DISTINCT l.a,l.k FROM
            (SELECT DISTINCT a FROM asked WHERE a IS NOT NULL AND b IS NOT NULL) d
            CROSS JOIN l WHERE l.a=d.a),
        rhs AS MATERIALIZED (SELECT DISTINCT r.b,r.k FROM
            (SELECT DISTINCT b FROM asked WHERE a IS NOT NULL AND b IS NOT NULL) d
            CROSS JOIN r WHERE r.b=d.b)
        SELECT q.i,q.a,q.b FROM asked q WHERE {mask} AND EXISTS
            (SELECT 1 FROM lhs JOIN rhs USING(k) WHERE lhs.a=q.a AND rhs.b=q.b)"""


def batch(db, keys, shape):
    """Keep correlated bindings together. Execute one query for each binding pattern."""
    answers = {key: [] for key in keys}
    if not answers:
        return answers
    db.execute("DELETE FROM asked")
    db.executemany(
        "INSERT INTO asked VALUES (?,?,?)", ((i, a, b) for i, (a, b) in enumerate(keys))
    )
    patterns = {(a is not None, b is not None) for a, b in keys}
    for has_a, has_b in sorted(patterns):
        mask = (
            ("q.a IS NOT NULL" if has_a else "q.a IS NULL")
            + " AND "
            + ("q.b IS NOT NULL" if has_b else "q.b IS NULL")
        )
        if shape == "demand" and has_a and has_b:
            sql = demanded_pair(db, mask)
        elif shape == "wide":
            tests = ["w.a=q.a"] if has_a else []
            if has_b:
                tests.append("w.b=q.b")
            condition = " AND ".join(tests) or "1"
            sql = f"SELECT q.i,w.a,w.b FROM asked q CROSS JOIN wide w WHERE {mask} AND {condition}"
        else:
            left, right = ("lf", "rf") if shape == "factors" else ("l", "r")
            if has_a and has_b:
                sql = f"""SELECT q.i,q.a,q.b FROM asked q WHERE {mask} AND EXISTS
                    (SELECT 1 FROM {left} l JOIN {right} r ON r.k=l.k
                     WHERE l.a=q.a AND r.b=q.b)"""
            else:
                # A right-bound call starts at r. A projected result has set semantics.
                first, second = (
                    (f"{right} r", f"{left} l")
                    if has_b
                    else (f"{left} l", f"{right} r")
                )
                restriction = (
                    "AND l.a=q.a" if has_a else ("AND r.b=q.b" if has_b else "")
                )
                sql = f"""SELECT DISTINCT q.i,l.a,r.b FROM asked q CROSS JOIN {first}
                    JOIN {second} ON r.k=l.k WHERE {mask} {restriction}"""
        for i, a, b in db.execute(sql):
            answers[keys[i]].append((a, b))
    return answers


class Strategy:
    def __init__(self, db, mode, preindexed, cache_limit):
        self.db = db
        self.mode = mode
        self.preindexed = preindexed
        self.limit = cache_limit
        self.cache = OrderedDict()
        self.weight = 0
        db.execute("CREATE TEMP TABLE asked(i INTEGER PRIMARY KEY,a INTEGER,b INTEGER)")
        self.built = False

    def invalidate(self):
        for table in ("wide", "lf", "rf", "supported"):
            self.db.execute(f"DROP TABLE IF EXISTS temp.{table}")
        self.cache.clear()
        self.weight = 0
        self.built = False

    def run(self, requests):
        db = self.db
        keys = list(dict.fromkeys(requests))
        metrics = dict(
            requests=len(requests),
            distinct_keys=len(keys),
            cache_hits=0,
            negative_hits=0,
            wide_rows_written=0,
            factor_rows_written=0,
        )
        if not keys and self.mode != "eager":
            return {}, metrics
        shape = "demand" if self.mode == "demand" else "direct"
        if self.mode == "eager":
            shape = "wide"
            if not self.built:
                db.execute(
                    "CREATE TEMP TABLE wide(a INTEGER,b INTEGER,PRIMARY KEY(a,b)) WITHOUT ROWID"
                )
                db.execute(
                    "INSERT OR IGNORE INTO wide SELECT l.a,r.b FROM l JOIN r ON r.k=l.k"
                )
                metrics["wide_rows_written"] = db.execute(
                    "SELECT changes()"
                ).fetchone()[0]
                db.execute("CREATE INDEX temp.by_b ON wide(b,a)")
        elif self.mode == "reduced" or (self.mode == "factors" and not self.preindexed):
            # Persist derived factors. Base factors already carrying both indexes need no work.
            shape = "factors"
            if not self.built:
                if self.mode == "reduced":
                    db.execute(
                        "CREATE TEMP TABLE supported(k INTEGER PRIMARY KEY) WITHOUT ROWID"
                    )
                    db.execute(
                        "INSERT INTO supported SELECT k FROM l INTERSECT SELECT k FROM r"
                    )
                    metrics["shared_keys_written"] = db.execute(
                        "SELECT changes()"
                    ).fetchone()[0]
                for dest, source, label in (("lf", "l", "a"), ("rf", "r", "b")):
                    db.execute(
                        f"CREATE TEMP TABLE {dest}({label} INTEGER,k INTEGER,PRIMARY KEY({label},k)) WITHOUT ROWID"
                    )
                    read = f"SELECT {label},k FROM {source}"
                    if self.mode == "reduced":
                        read = f"SELECT t.{label},t.k FROM supported s CROSS JOIN {source} t WHERE t.k=s.k"
                    db.execute(f"INSERT OR IGNORE INTO {dest} {read}")
                    metrics["factor_rows_written"] += db.execute(
                        "SELECT changes()"
                    ).fetchone()[0]
                    db.execute(f"CREATE INDEX temp.{dest}_key ON {dest}(k,{label})")
        self.built = True
        if self.mode != "tabled":
            return batch(db, keys, shape), metrics

        # A cache entry certifies completeness for its exact binding mask, including a miss.
        answers = {}
        missing = []
        for key in keys:
            if key in self.cache:
                answers[key] = self.cache[key]
                self.cache.move_to_end(key)
                metrics["cache_hits"] += 1
                metrics["negative_hits"] += int(not answers[key])
            else:
                missing.append(key)
        for key, rows in batch(db, missing, "direct").items():
            answers[key] = rows
            weight = 1 + len(rows)
            if weight > self.limit:
                continue
            while self.weight + weight > self.limit:
                _, expired = self.cache.popitem(last=False)
                self.weight -= 1 + len(expired)
            self.cache[key] = rows
            self.weight += weight
        metrics["cache_weight"] = self.weight
        return answers, metrics
