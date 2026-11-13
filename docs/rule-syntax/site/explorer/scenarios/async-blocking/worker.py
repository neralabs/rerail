import asyncio
import time
from time import sleep


async def poll(queue):
    while queue.empty():
        time.sleep(0.5)
    return queue.get_nowait()


async def backoff(attempt):
    sleep(2 ** attempt)


async def polite(queue):
    while queue.empty():
        await asyncio.sleep(0.5)


def sync_wait():
    time.sleep(1)
