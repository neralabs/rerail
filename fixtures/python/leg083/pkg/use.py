from pathlib import Path

from pkg.reads import Holder, Txn


def eager(txn: Txn):
    return txn.select_one_lazy(1).resolve()


def through(txn: Txn):
    return eager(txn)


def decorated(txn: Txn):
    return txn.count_lazy().resolve()


def attribute(h: Holder):
    return h.rows.resolve()


def chained(h: Holder):
    return h.txn.select_one_lazy(2).resolve_async()


def quoted(txn: Txn):
    return txn.query_lazy().resolve()


def paths(p: Path):
    return p.resolve()


def lookup(catalog, ref):
    return catalog.resolve(ref)


def untyped(x):
    return x.resolve()
