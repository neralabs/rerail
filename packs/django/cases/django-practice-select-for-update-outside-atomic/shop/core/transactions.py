from contextlib import contextmanager

from ..core.tracing import traced_transaction


@contextmanager
def commit_on_errors():
    with traced_transaction():
        yield
