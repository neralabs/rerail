import asyncio
import contextlib
import logging
from asyncio import CancelledError
from contextlib import suppress

logger = logging.getLogger(__name__)


async def read_then_log(conn):
    with contextlib.suppress(asyncio.CancelledError):  # caught
        await conn.read()
    logger.info("go on")


async def read_imported(conn):
    with suppress(CancelledError, OSError):  # caught
        await conn.read()
    await conn.close()


async def read_broad(conn):
    with suppress(BaseException):  # caught
        await conn.read()
    return conn.buffer


async def read_each(conns):
    # The loop goes on to the next connection after a cancel.
    for conn in conns:
        with suppress(asyncio.CancelledError):  # caught
            await conn.read()


async def drain(task):
    task.cancel()
    with contextlib.suppress(asyncio.CancelledError):
        await task
    logger.info("drained")


class Worker:
    def __init__(self):
        self._task = None

    def _stop(self):
        self._task.cancel()

    async def stop(self):
        self._stop()
        with suppress(asyncio.CancelledError):
            await self._task
        logger.info("stopped")


async def read_last(conn):
    logger.info("reading")
    with suppress(asyncio.CancelledError):
        await conn.read()


async def read_ignoring_errors(conn):
    with suppress(Exception):
        await conn.read()
    logger.info("go on")


async def read_ignoring_timeouts(conn):
    with suppress(asyncio.TimeoutError):
        await conn.read()
    logger.info("go on")


async def close_on_cancel(conn):
    try:
        await conn.read()
    except asyncio.CancelledError:
        with suppress(asyncio.CancelledError):
            await asyncio.shield(conn.close())
        raise


def sync_close(conn):
    with suppress(asyncio.CancelledError):
        conn.close()
    logger.info("closed")


async def no_await(conn):
    with suppress(asyncio.CancelledError):
        conn.close_nowait()
    logger.info("closed")
