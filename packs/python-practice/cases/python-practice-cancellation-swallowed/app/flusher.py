import asyncio
import logging

from app.background import safe_create_task

logger = logging.getLogger(__name__)


class Flusher:
    def __init__(self, sink):
        self.sink = sink
        self._task = None

    def start(self):
        # Spawned through the helper in another module: _periodic_flush only ever runs as a task.
        self._task = safe_create_task(self._periodic_flush(), label="flush")

    async def _periodic_flush(self):
        while True:
            try:
                await asyncio.sleep(1)
                await self.sink.flush()
            except asyncio.CancelledError:
                break
            except Exception:
                logger.exception("flush failed")

    async def flush_now(self):
        # Awaited directly by its callers: the swallow reaches them.
        try:
            await self.sink.flush()
        except asyncio.CancelledError:  # caught
            logger.info("flush cancelled")
