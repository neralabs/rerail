import contextlib
import os
import shutil
import tempfile
from contextlib import asynccontextmanager, contextmanager


@contextmanager
def held(lock):
    lock.acquire()
    yield lock  # caught
    lock.release()


@contextlib.contextmanager
def scratch_dir():
    path = tempfile.mkdtemp()
    yield path  # caught
    shutil.rmtree(path)


@contextmanager
def scratch_file(path):
    open(path, "w").close()
    yield path  # caught
    os.remove(path)


@asynccontextmanager
async def connection(pool):
    conn = await pool.acquire()
    yield conn  # caught
    await conn.close()


@contextmanager
def rows(conn):
    cur = conn.cursor()
    for row in cur:
        yield row  # caught
    cur.close()


@contextmanager
def maybe_connected(conn):
    if conn is None:
        yield None
        return
    yield conn  # caught
    conn.close()


class Worker:
    @contextmanager
    def running(self):
        self.pool.start()
        yield self.pool  # caught
        self.pool.shutdown(wait=True)


@contextmanager
def temp_file():
    fd, path = tempfile.mkstemp()
    yield path  # caught
    os.close(fd)
