import asyncio
import logging

logger = logging.getLogger(__name__)
_reader = None


def _cancel_reader():
    _reader.cancel()


async def stop_reader():
    # The cancel is in a module-level helper of the same module.
    _cancel_reader()
    try:
        await _reader
    except asyncio.CancelledError:
        pass
    logger.info("reader stopped")


class Pump:
    def __init__(self):
        self.tasks = {}
        self._task = None
        self._other = None

    def _stop(self):
        self._task.cancel()

    def _shutdown(self):
        logger.info("shutting down")
        self._stop()

    def _stop_other(self):
        self._other.cancel()

    async def close(self):
        # The cancel is in a helper of the class.
        self._stop()
        try:
            await self._task
        except asyncio.CancelledError:
            pass
        logger.info("closed")

    async def close_deep(self):
        # Two helpers down.
        self._shutdown()
        try:
            await asyncio.wait_for(self._task, timeout=5)
        except asyncio.CancelledError:
            pass
        logger.info("closed")

    async def close_one(self, k):
        # A drain through a subscript.
        self.tasks[k].cancel()
        try:
            await self.tasks[k]
        except asyncio.CancelledError:
            pass
        logger.info("closed %s", k)

    async def close_wrong_key(self, a, b):
        self.tasks[a].cancel()
        try:
            await self.tasks[b]
        except asyncio.CancelledError:  # caught
            pass
        logger.info("closed %s", b)

    async def close_other(self):
        # The helper cancels another task than the one awaited.
        self._stop_other()
        try:
            await self._task
        except asyncio.CancelledError:  # caught
            pass
        logger.info("closed")

    async def close_late(self):
        # The cancel comes after the await: the CancelledError caught is this function's own.
        try:
            await self._task
        except asyncio.CancelledError:  # caught
            pass
        self._stop()


class SubPump(Pump):
    async def restart(self):
        # The helper is inherited from the base class.
        self._stop()
        try:
            await self._task
        except asyncio.CancelledError:
            pass
        await self.start()

    async def start(self):
        await asyncio.sleep(0)
