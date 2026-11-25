import asyncio
import logging

from exceptiongroup import BaseExceptionGroup as Group

logger = logging.getLogger(__name__)


class ShutdownErrors(BaseExceptionGroup):
    """Every error a shutdown met, the caller's own among them."""


class Metering:
    async def __aexit__(self, exc_type, exc_value, traceback):
        # polar's shape: the body's error and the drain's error travel on together.
        try:
            await self._close()
        except BaseException as error:
            if exc_value is not None:
                raise BaseExceptionGroup("body and delivery failed", [exc_value, error]) from None
            raise

    async def _close(self):
        await asyncio.sleep(0)


async def close_through_alias(conn, pending):
    try:
        await conn.close()
    except BaseException as exc:
        if pending:
            raise Group("close failed", (exc, pending[0]))
        raise


async def close_through_list(conn, cleanup):
    try:
        await conn.close()
    except BaseException as exc:
        try:
            await cleanup()
        except Exception as cleanup_error:
            errors = [exc, cleanup_error]
            raise BaseExceptionGroup("close and cleanup failed", errors)
        raise


async def close_through_append(conns):
    failures = []
    for conn in conns:
        try:
            await conn.close()
        except BaseException as exc:
            failures.append(exc)
            raise BaseExceptionGroup("close failed", failures)


async def close_through_subclass(conn):
    try:
        await conn.close()
    except asyncio.CancelledError as exc:
        raise ShutdownErrors("shutdown", [exc])


async def plain_group(conn):
    # ExceptionGroup refuses a BaseException: a cancellation turns into TypeError here.
    try:
        await conn.close()
    except BaseException as exc:  # caught
        raise ExceptionGroup("close failed", [exc])


async def group_without_it(conn, other):
    # The group carries another error; the cancellation itself is dropped.
    try:
        await conn.close()
    except asyncio.CancelledError:  # caught
        raise BaseExceptionGroup("close failed", [other])


async def group_of_a_different_name(conn, exc):
    try:
        await conn.close()
    except asyncio.CancelledError as error:  # caught
        logger.info("cancelled: %s", error)
        raise BaseExceptionGroup("close failed", [exc])
