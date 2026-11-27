from .decorators import contextmanager


@contextmanager
def held(lock):
    lock.acquire()
    yield lock
    lock.release()
