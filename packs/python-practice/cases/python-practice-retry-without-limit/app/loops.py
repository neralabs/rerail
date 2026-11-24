import asyncio
import logging
import sys
import time

logger = logging.getLogger(__name__)


class PermanentError(Exception):
    pass


def connect(db, dsn):
    while True:  # caught
        try:
            conn = db.connect(dsn)
            break
        except ConnectionError:
            logger.warning("database not ready, retrying")
            time.sleep(1)
    return conn


def fetch_status(client):
    while 1:  # caught
        try:
            return client.get("/status")
        except Exception as exc:
            logger.info("status failed: %s", exc)
            continue


async def open_channel(client):
    while True:  # caught
        try:
            channel = await client.open()
        except OSError:
            await asyncio.sleep(1)
        else:
            break
    return channel


def fetch_or_notify(client, hooks):
    while True:  # caught
        try:
            return client.get("/items")
        except Exception:
            for hook in hooks:
                if hook():
                    break


def fetch_transient(client):
    while True:  # caught
        try:
            return client.get("/items")
        except PermanentError:
            raise
        except TimeoutError:
            time.sleep(1)


def fetch_with_callback(client):
    while True:  # caught
        try:
            return client.get("/items")
        except Exception:

            def report():
                raise RuntimeError("never called here")

            logger.info("retrying")


# Bounded: a counter, a deadline, a range, an exit in the handler.
def fetch_counted(client, attempts=5):
    attempt = 0
    while True:
        try:
            return client.get("/items")
        except Exception:
            attempt += 1
            if attempt >= attempts:
                raise
            time.sleep(attempt)


def fetch_until_deadline(client):
    deadline = time.monotonic() + 30
    while True:
        try:
            return client.get("/items")
        except Exception:
            if time.monotonic() > deadline:
                raise TimeoutError("gave up after 30s") from None
            time.sleep(1)


def fetch_deadline_at_top(client):
    deadline = time.monotonic() + 30
    while True:
        if time.monotonic() > deadline:
            raise TimeoutError("gave up after 30s")
        try:
            return client.get("/items")
        except Exception:
            time.sleep(1)


def fetch_counted_after(client):
    tries = 0
    while True:
        try:
            result = client.get("/items")
            break
        except Exception:
            tries += 1
            time.sleep(1)
        if tries > 3:
            return None
    return result


def fetch_range(client):
    for _attempt in range(5):
        try:
            return client.get("/items")
        except Exception:
            time.sleep(1)
    raise RuntimeError("gave up")


def fetch_or_exit(client):
    while True:
        try:
            return client.get("/items")
        except KeyboardInterrupt:
            sys.exit(1)
        except Exception:
            logger.exception("fetch failed")
            sys.exit(2)


def fetch_once_more(client):
    while True:
        try:
            return client.get("/items")
        except Exception:
            logger.exception("fetch failed")
            return None


# Not a retry: a worker that is meant to run forever, with no exit on success.
def work(queue, handle):
    while True:
        try:
            job = queue.get()
            handle(job)
        except Exception:
            logger.exception("job failed")


async def poll(source, sink):
    while True:
        try:
            for item in await source.fetch():
                if item is None:
                    break
                await sink.put(item)
        except Exception:
            logger.exception("poll failed")
        await asyncio.sleep(5)


# Not this loop's retry: the while has a real condition.
def drain(queue):
    while queue:
        try:
            return queue.pop()
        except IndexError:
            time.sleep(0.1)


# Not a retry: a writer that stops on a signal or a sentinel, and forwards each failure.
def post_writer(stop_event, outbox, errors, send):
    while True:
        try:
            if stop_event.is_set():
                break
            message = outbox.get(timeout=1)
            if message is None:
                break
            send(message)
        except Exception as exc:
            errors.put(exc)


# Not a retry: an interactive session that reconnects; the try body only raises, it never finishes.
async def chat(connect, ask):
    while True:
        try:
            async with connect() as ws:
                while True:
                    try:
                        await ws.send(ask())
                    except ConnectionError:
                        raise
        except TimeoutError:
            continue


def save_with_lock(lock, store, item):
    while True:  # caught
        try:
            with lock:
                store.save(item)
                break
        except OSError:
            time.sleep(1)
