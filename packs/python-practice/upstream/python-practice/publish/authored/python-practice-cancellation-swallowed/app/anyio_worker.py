import asyncio
import logging

import anyio
import trio
from anyio import move_on_after

logger = logging.getLogger(__name__)


class Buffered:
    def __init__(self, queue, sink):
        self._queue = queue
        self._sink = sink
        self._failure = None

    async def _flush_task(self, *, task_status):
        # polar's shape: anyio re-delivers the scope's cancellation at the next checkpoint.
        failure = None
        cancel_scope = anyio.CancelScope(shield=True)
        try:
            with cancel_scope:
                task_status.started(cancel_scope)
                await self._sink.flush(await self._queue.receive())
        except BaseException as error:
            failure = error
            self._failure = error
        finally:
            if failure is not None:
                try:
                    with anyio.CancelScope(shield=True):
                        await self._sink.drop()
                except BaseException as callback_error:
                    self._failure = callback_error
        await self._sink.close()

    async def _drain(self):
        with move_on_after(5):
            try:
                await self._sink.flush(None)
            except:
                logger.info("drain stopped")
        await self._sink.close()

    async def _named(self):
        # asyncio's CancelledError named outright: still reported in anyio code.
        with anyio.move_on_after(5):
            try:
                await self._sink.flush(None)
            except asyncio.CancelledError:  # caught
                logger.info("flush stopped")
        await self._sink.close()


async def nursery_child(stream):
    try:
        await stream.send_all(b"x")
    except BaseException:
        logger.info("send stopped")
    await trio.sleep(0)


async def no_anyio_here(conn):
    # The module imports anyio, but this coroutine uses none of it: asyncio code.
    try:
        await conn.read()
    except BaseException:  # caught
        logger.exception("read failed")
    await conn.close()
