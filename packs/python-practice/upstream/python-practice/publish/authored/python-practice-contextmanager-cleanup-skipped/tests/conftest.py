from contextlib import contextmanager

import pytest


@pytest.fixture
def conn(pool):
    conn = pool.acquire()
    yield conn
    conn.close()


@contextmanager
def held(lock):
    lock.acquire()
    yield lock
    lock.release()
