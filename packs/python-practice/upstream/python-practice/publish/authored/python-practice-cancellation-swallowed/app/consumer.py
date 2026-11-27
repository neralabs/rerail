import asyncio
import logging

logger = logging.getLogger(__name__)


class Consumer:
    """Runs its own event loop in a thread, one task per producer."""

    def __init__(self, loop, producers):
        self._loop = loop
        self.producers = producers

    def start(self):
        return asyncio.run_coroutine_threadsafe(self._main(), loop=self._loop)

    async def _main(self):
        # The thread's top coroutine: a cancelled main is the shutdown.
        tasks = [self._loop.create_task(self._pump(p)) for p in self.producers]
        try:
            await asyncio.gather(*tasks)
        except asyncio.CancelledError:
            pass
        finally:
            self._loop.stop()

    async def _pump(self, producer):
        # Only ever a task's top coroutine, and the handler ends it: the task stops when cancelled.
        try:
            await producer.consume()
        except asyncio.CancelledError:
            logger.info("producer %s stopped", producer)

    def start_retry(self):
        return self._loop.create_task(self._retry_loop())

    async def _retry_loop(self):
        # A task's top coroutine whose loop the handler leaves: the task ends.
        while True:
            try:
                await self._connect()
            except asyncio.CancelledError:
                logger.info("retry loop cancelled")
                break
            except OSError:
                await asyncio.sleep(1)

    async def _connect(self):
        await asyncio.sleep(0)

    def start_ticker(self):
        return self._loop.create_task(self._ticker())

    async def _ticker(self):
        # A task's top coroutine, but the handler falls through and the loop goes on.
        while True:
            try:
                await self._connect()
            except asyncio.CancelledError:  # caught
                logger.info("tick cancelled")
            await asyncio.sleep(1)


async def process_request(handler, request, send):
    response = await handler.get_response(request)
    try:
        await handler.send_response(response, send)
    except asyncio.CancelledError:  # caught
        # A task, but it then returns the response: whoever reads the task's result gets it as a success.
        pass
    return response


async def serve(handler, request, send):
    task = asyncio.create_task(process_request(handler, request, send))
    done, _pending = await asyncio.wait([task], timeout=30)
    return done
