import logging
import os
import tempfile
import time
from contextlib import asynccontextmanager, contextmanager

from fastapi import FastAPI
from starlette.applications import Starlette

logger = logging.getLogger(__name__)


@contextmanager
def held(lock):
    lock.acquire()
    try:
        yield lock
    finally:
        lock.release()


@contextmanager
def held_with(lock):
    with lock:
        yield lock


@contextmanager
def transaction(conn):
    try:
        yield conn
    except Exception:
        conn.rollback()
        raise
    conn.commit()


@contextmanager
def batch(queue, session):
    items = []
    yield items
    queue.extend(items)
    session.flush()
    session.commit()
    logger.info("flushed %d items", len(items))


@contextmanager
def maybe_locked(lock):
    if lock is None:
        yield
        return
    lock.acquire()
    try:
        yield
    finally:
        lock.release()


@contextmanager
def counted(stats):
    yield
    stats.reset()


@contextmanager
def marked(job):
    yield job
    job.done = True
    job.status = job.previous_status


@contextmanager
def opened(path):
    handle = open(path)
    yield handle
    print("done")


@contextmanager
def outer(conn):
    def drain():
        yield 1
        conn.close()

    yield drain


@contextmanager
def chdir_once(path):
    yield path
    os.chdir(path)


@asynccontextmanager
async def lifespan(app):
    pool = await app.state.create_pool()
    yield
    await pool.close()


@asynccontextmanager
async def startup(app):
    app.state.client = app.state.make_client()
    yield
    await app.state.client.aclose()


api = FastAPI(lifespan=lifespan)
site = Starlette(lifespan=startup)


@contextmanager
def checkout(url):
    local_path = tempfile.TemporaryDirectory()
    clone(url, local_path.name)
    yield local_path.name
    local_path.cleanup()


@contextmanager
def timed(metrics, notify):
    start = time.monotonic()
    notify("start")
    yield
    notify("end")
    metrics.observe(start)


@contextmanager
def tracked(job):
    job.status = "running"
    job.finished = None
    yield job
    job.status = "done"
    job.finished = job.clock()


def clone(url, path):
    return (url, path)
