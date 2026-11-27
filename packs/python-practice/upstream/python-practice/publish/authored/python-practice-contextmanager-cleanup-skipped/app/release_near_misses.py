import logging
import threading
from contextlib import contextmanager, suppress

_context = threading.local()
_lock = threading.Lock()
log = logging.getLogger(__name__)


@contextmanager
def forget(cache):
    yield cache
    del cache.entries


@contextmanager
def scoped_reported(registry, key, value):
    value.ready = True
    yield value
    del registry[key]


@contextmanager
def progress(bar):
    bar.update(0)
    yield bar
    bar.update(100)


@contextmanager
def status(reporter):
    reporter.send(1)
    yield
    reporter.send(1)


@contextmanager
def held_swallowing():
    _lock.acquire()
    try:
        yield
    except Exception:
        log.exception("block failed")
    _lock.release()


@contextmanager
def held_bare():
    _lock.acquire()
    try:
        yield
    except:  # noqa: E722
        log.exception("block failed")
    _lock.release()


@contextmanager
def held_suppressed():
    _lock.acquire()
    with suppress(Exception):
        yield
    _lock.release()


@contextmanager
def held_guarded():
    _lock.acquire()
    try:
        yield
    finally:
        _lock.release()


@contextmanager
def looked_up(cache, key):
    cache.get(key)
    yield
    cache.pop(key)


@contextmanager
def drained(queue):
    yield queue
    queue.pop()


@contextmanager
def committed(session, items):
    session.add_all(items)
    yield
    session.commit()
