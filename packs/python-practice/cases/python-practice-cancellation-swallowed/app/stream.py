import asyncio
import logging
from asyncio import CancelledError
from asyncio.exceptions import CancelledError as ExcCancelledError

logger = logging.getLogger(__name__)


async def fetch_page(client, url):
    return await client.get(url)


async def poll_forever(client, url):
    while True:
        try:
            await fetch_page(client, url)
        except asyncio.CancelledError:  # caught
            logger.info("poll cancelled")
        await asyncio.sleep(5)


async def download_all(client, urls):
    pages = []
    for url in urls:
        try:
            pages.append(await fetch_page(client, url))
        except asyncio.CancelledError:  # caught
            logger.debug("interrupted, returning what we have")
            break
    return pages


async def read_one(queue):
    try:
        return await queue.get()
    except CancelledError:  # caught
        return None


async def send(conn, payload):
    try:
        await conn.write(payload)
    except ExcCancelledError:  # caught
        raise ConnectionError("send aborted")


async def drain_queue(queue):
    while not queue.empty():
        try:
            await queue.get()
        except (OSError, asyncio.CancelledError):  # caught
            continue


async def guarded(client, url):
    try:
        await fetch_page(client, url)
    except BaseException:  # caught
        logger.exception("fetch failed")


async def bare(client, url):
    try:
        await fetch_page(client, url)
    except:  # caught
        pass


class Stream:
    def __init__(self, conn):
        self.conn = conn
        self.closing = False

    async def flush(self):
        try:
            async with self.conn.lock:
                self.conn.flush_nowait()
        except asyncio.CancelledError:  # caught
            if self.closing:
                return
            raise

    async def listen(self):
        try:
            async for msg in self.conn:
                logger.info(msg)
        except asyncio.CancelledError:  # caught
            pass


async def stop_all(tasks):
    # asyncio.wait() never raises a child's CancelledError: this one can only be our own.
    [t.cancel() for t in tasks]
    try:
        await asyncio.wait(tasks)
    except asyncio.CancelledError:  # caught
        logger.debug("ignoring")


def make_handler(conn):
    async def handler():
        try:
            await conn.write(b"x")
        except asyncio.CancelledError:  # caught
            pass

    return handler


async def heartbeat(conn):
    # Started as a task, but the handler falls through to more awaits: it keeps running after cancel().
    try:
        await conn.ping()
    except asyncio.CancelledError:  # caught
        logger.info("heartbeat cancelled")
    await conn.close()


def start_heartbeat(conn):
    return asyncio.create_task(heartbeat(conn))


async def pump_once(conn):
    # Awaited directly as well as spawned: the caller resumes as if nothing was cancelled.
    try:
        await conn.recv()
    except asyncio.CancelledError:  # caught
        return None


async def pump_twice(conn):
    background = asyncio.create_task(pump_once(conn))
    await pump_once(conn)
    return background


async def record_only(conn, journal):
    # The flag is written down but nothing raises it again.
    cancelled = False
    try:
        await conn.read()
    except asyncio.CancelledError:  # caught
        cancelled = True
    await journal.write({"cancelled": cancelled})
