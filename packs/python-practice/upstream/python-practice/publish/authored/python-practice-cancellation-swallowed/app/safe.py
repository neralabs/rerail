import asyncio
import contextlib
import logging

logger = logging.getLogger(__name__)


async def work():
    await asyncio.sleep(1)


async def drain_task(task):
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        pass


class Worker:
    def __init__(self):
        self._task = None
        self._tasks = []

    async def stop(self):
        self._task.cancel()
        try:
            await self._task
        except asyncio.CancelledError:
            logger.debug("worker stopped")

    async def stop_all(self):
        for t in self._tasks:
            t.cancel()
        try:
            await asyncio.gather(*self._tasks)
        except asyncio.CancelledError:
            pass

    async def stop_with_deadline(self):
        self._task.cancel()
        try:
            await asyncio.wait_for(self._task, timeout=5)
        except (asyncio.CancelledError, asyncio.TimeoutError):
            pass


async def wait_child(child):
    try:
        return await child
    except asyncio.CancelledError:
        if asyncio.current_task().cancelling():
            raise
        return "child cancelled"


async def cleanup_then_raise(conn):
    try:
        await conn.read()
    except asyncio.CancelledError:
        await conn.close()
        raise


async def raise_in_finally(conn):
    try:
        await conn.read()
    except asyncio.CancelledError:
        try:
            await conn.close()
        finally:
            raise


async def raise_alias(conn):
    try:
        await conn.read()
    except asyncio.CancelledError as exc:
        logger.info("cancelled")
        raise exc


async def raise_fresh(conn):
    try:
        await conn.read()
    except asyncio.CancelledError:
        await conn.close()
        raise asyncio.CancelledError()


async def both_branches(conn, strict):
    try:
        await conn.read()
    except BaseException:
        if strict:
            await conn.abort()
            raise
        else:
            raise


async def mixed(conn):
    try:
        await conn.read()
    except (Exception, asyncio.CancelledError) as exc:
        await conn.record_failure(str(exc))
        if isinstance(exc, asyncio.CancelledError):
            raise
        logger.warning("read failed")


def _surface(exc):
    logger.info("surfacing")
    raise exc


async def delegated(conn):
    try:
        await conn.read()
    except asyncio.CancelledError as exc:
        _surface(exc)


async def broad(conn):
    try:
        await conn.read()
    except Exception:
        logger.exception("read failed")


async def no_await(fut):
    try:
        return fut.result()
    except asyncio.CancelledError:
        return None


def sync_server(server):
    try:
        server.run()
    except asyncio.CancelledError:
        pass


async def with_sync_callback(fut):
    def on_done(f):
        try:
            f.result()
        except asyncio.CancelledError:
            pass

    fut.add_done_callback(on_done)
    await fut


async def suppressed(conn):
    with contextlib.suppress(Exception):
        await conn.read()


async def nested_cleanup(conn):
    try:
        await conn.read()
    except asyncio.CancelledError:
        try:
            await asyncio.shield(conn.close())
        except asyncio.CancelledError:
            pass
        raise


async def record_then_raise(conn, journal):
    cancelled = False
    try:
        result = await conn.read()
    except asyncio.CancelledError:
        result = None
        cancelled = True
    await journal.write(result)
    if cancelled:
        raise asyncio.CancelledError
    return result


class RelayInterrupted(Exception):
    pass


async def relay(stream):
    # Only OSError and RelayInterrupted are converted; a cancellation takes the `else: raise`.
    try:
        return await stream.receive()
    except BaseException as error:
        await stream.close()
        if isinstance(error, RelayInterrupted):
            interrupted = error
        elif isinstance(error, (OSError, TimeoutError)):
            interrupted = RelayInterrupted(str(error))
        else:
            raise
        raise interrupted from error
