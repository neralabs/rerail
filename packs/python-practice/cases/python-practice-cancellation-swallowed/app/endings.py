import asyncio
import functools
import logging

from fastapi import APIRouter, WebSocket, WebSocketDisconnect
from starlette.responses import StreamingResponse

logger = logging.getLogger(__name__)
router = APIRouter()


async def events(queue):
    # Handed to Starlette: the handler is the generator's last act, nothing of the run awaits it.
    try:
        while True:
            yield await queue.get()
    except asyncio.CancelledError:
        logger.info("client went away")


async def events_with_goodbye(queue):
    try:
        while True:
            yield await queue.get()
    except asyncio.CancelledError:  # caught
        yield "event: bye\n\n"


async def events_then_summary(queue):
    # The generator yields after the handler: it keeps producing after the cancel.
    try:
        while True:
            yield await queue.get()
    except asyncio.CancelledError:  # caught
        logger.info("stream cancelled")
    yield "event: summary\n\n"


def stream(queue):
    return StreamingResponse(events(queue), media_type="text/event-stream")


def stream_variants(queue):
    first = StreamingResponse(events_with_goodbye(queue))
    second = StreamingResponse(content=events_then_summary(queue))
    return first, second


@router.websocket("/ws")
async def echo(websocket: WebSocket):
    # Registered with the router: the framework calls it, and nothing runs after the handler.
    await websocket.accept()
    try:
        while True:
            await websocket.send_text(await websocket.receive_text())
    except (WebSocketDisconnect, asyncio.CancelledError):
        logger.info("closed")


@router.websocket("/ws/close")
async def echo_then_close(websocket: WebSocket):
    try:
        await websocket.send_text(await websocket.receive_text())
    except asyncio.CancelledError:  # caught
        await websocket.close()


class Refresher:
    def __init__(self, scheduler, client):
        self.client = client
        scheduler.add_job(self.refresh, "interval", seconds=60)

    async def refresh(self):
        # Passed uncalled to a scheduler: a job's last act, inside the client's session.
        async with self.client.session() as session:
            try:
                await session.refresh()
            except asyncio.CancelledError:
                logger.info("refresh cancelled")
        return None


async def download(client, scheduler):
    # Handed off once, but also awaited below: that caller gets the partial list as a success.
    pages = []
    while True:
        try:
            pages.append(await client.next_page())
        except asyncio.CancelledError:  # caught
            logger.debug("interrupted, returning what we have")
            break
    return pages


async def sync_all(client, scheduler):
    scheduler.submit(download(client, scheduler))
    return await download(client, scheduler)


async def watch(client):
    try:
        await client.watch()
    except asyncio.CancelledError:  # caught
        logger.info("watch cancelled")


async def watch_all(client, scheduler):
    # Handed off once, but gather() also awaits watch() for this function, which then goes on.
    scheduler.submit(watch(client))
    pending = asyncio.gather(watch(client), client.ping())
    await pending
    logger.info("watched")


async def poll(client):
    try:
        await client.poll()
    except asyncio.CancelledError:  # caught
        logger.info("poll cancelled")


async def guarded(coro):
    try:
        await coro
    finally:
        logger.info("done")


def start_poll(client, scheduler):
    # Handed off once, but guarded() awaits its argument and returns to whoever awaits it.
    scheduler.submit(poll(client))
    return asyncio.ensure_future(guarded(poll(client)))


async def fetch(client):
    try:
        await client.fetch()
    except asyncio.CancelledError:  # caught
        logger.info("fetch cancelled")


async def fetch_with_retry(client, retrying):
    # An outside helper that is awaited: it awaits fetch and returns here.
    await retrying.call(functools.partial(fetch, client))
    logger.info("fetched")


async def notify(client):
    try:
        await client.notify()
    except asyncio.CancelledError:  # caught
        logger.info("notify cancelled")


def wrap(handler):
    @functools.wraps(handler)
    async def wrapper(*args):
        # freqtrade's shape: a wrapper around every command, decorated by functools.wraps, not registered.
        try:
            return await handler(*args)
        except BaseException:  # caught
            logger.exception("command failed")

    return wrapper
