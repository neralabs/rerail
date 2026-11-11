from pathlib import Path

from l2055_lazy_result.lazy_result import LazyResult
from l2055_lazy_result.lazy_seq import LazySeq


def lazy[F](fn: F) -> F:
    def wrapper(*args, **kwargs):
        return fn(*args, **kwargs)
    return wrapper


class Txn:
    def select_one_lazy(self, key: int) -> LazyResult[int | None]: ...

    def query_lazy(self) -> "LazySeq[int]": ...

    @lazy
    def count_lazy(self) -> LazyResult[int]: ...

    def base(self, path: Path) -> Path:
        return path.resolve()


class Holder:
    rows: LazySeq[int]
    txn: Txn
