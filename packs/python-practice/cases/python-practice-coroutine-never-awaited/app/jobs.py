import asyncio
import threading

from vendor.base import RemoteBase


class JobQueue:
    """Runs sync and async callables alike: submit awaits a coroutine function."""

    def submit(self, fn, *args):
        if asyncio.iscoroutinefunction(fn):
            return asyncio.ensure_future(fn(*args))
        return fn(*args)


class Base:
    async def run(self):
        await asyncio.sleep(0)

    def start(self):
        self.run()

    def start_thread(self):
        threading.Thread(target=self.run).start()


class Job(Base):
    def run(self):
        return 1


class Remote(RemoteBase):
    def close(self):
        self.flush()

    def close_later(self):
        threading.Thread(target=self.flush).start()


class Pinger:
    async def ping(self):
        await asyncio.sleep(0)

    def kick(self):
        self.ping()  # caught
