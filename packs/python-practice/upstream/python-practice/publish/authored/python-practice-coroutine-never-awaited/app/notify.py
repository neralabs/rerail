import asyncio
import functools
import threading

from asgiref.sync import async_to_sync


async def send_mail(user):
    await asyncio.sleep(0)


async def stream():
    yield 1


async def cleanup(*args):
    await asyncio.sleep(0)


def plain(user):
    return user


def run_sync(fn):
    @functools.wraps(fn)
    def wrapper(*args):
        return asyncio.run(fn(*args))

    return wrapper


@async_to_sync
async def sync_send(user):
    await send_mail(user)


@run_sync
async def blocking_send(user):
    await send_mail(user)


class Notifier:
    async def flush(self):
        await asyncio.sleep(0)

    def close(self):
        self.flush()  # caught

    def close_later(self):
        threading.Thread(target=self.flush).start()  # caught
