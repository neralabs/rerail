import contextlib
import logging
import signal
import socket
import sys
import threading
from contextlib import contextmanager, suppress

_context = threading.local()
_lock = threading.Lock()
_handlers = []
log = logging.getLogger(__name__)


@contextmanager
def report_api_call(request):
    root = False
    if not hasattr(_context, "api_call"):
        _context.api_call, root = object(), True
    yield _context.api_call  # caught
    if root:
        del _context.api_call


@contextmanager
def registered(name, module):
    sys.modules[name] = module
    yield module  # caught
    del sys.modules[name]


@contextmanager
def tagged(obj, value):
    setattr(obj, "tag", value)
    yield obj  # caught
    delattr(obj, "tag")


@contextmanager
def time_limit(seconds):
    signal.signal(signal.SIGALRM, _raise)
    signal.alarm(seconds)
    yield  # caught
    signal.alarm(0)


@contextmanager
def default_handler(handler):
    signal.signal(signal.SIGALRM, handler)
    yield  # caught
    signal.signal(signal.SIGALRM, signal.SIG_DFL)


@contextmanager
def quiet():
    logging.disable(logging.CRITICAL)
    yield  # caught
    logging.disable(logging.NOTSET)


@contextmanager
def socket_timeout(seconds):
    socket.setdefaulttimeout(seconds)
    yield  # caught
    socket.setdefaulttimeout(None)


@contextmanager
def on_path(path):
    sys.path.insert(0, path)
    yield  # caught
    sys.path.remove(path)


@contextmanager
def on_path_end(path):
    sys.path.append(path)
    yield  # caught
    sys.path.pop()


@contextmanager
def capture(handler):
    log.addHandler(handler)
    yield handler  # caught
    log.removeHandler(handler)


@contextmanager
def hooked(handler):
    _handlers.append(handler)
    yield  # caught
    _handlers.remove(handler)


@contextmanager
def scoped(registry, key, value):
    registry[key] = value
    yield value  # caught
    registry.pop(key, None)


@contextmanager
def held_narrow():
    _lock.acquire()
    try:
        yield  # caught
    except ValueError:
        log.warning("bad value")
        raise
    _lock.release()


@contextmanager
def held_after_finally(conn):
    try:
        yield conn  # caught
    finally:
        log.info("block done")
    conn.close()


@contextmanager
def opened_then_removed(path):
    with open(path) as handle:
        yield handle  # caught
    path.unlink()


@contextlib.asynccontextmanager
async def remote_user(store, user):
    previous = await store.get_user()
    await store.set_user(user)
    yield  # caught
    await store.set_user(previous)


def _raise(*args):
    raise TimeoutError
