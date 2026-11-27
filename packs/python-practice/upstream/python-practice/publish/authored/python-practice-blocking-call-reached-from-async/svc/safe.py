import asyncio
import functools

import anyio
import trio
from asgiref.sync import sync_to_async

from svc.lookalikes import local_get, make_client
from svc.services import http, shell
from svc.services.http import fetch


async def offloaded(url):
    a = await asyncio.to_thread(fetch, url)
    loop = asyncio.get_running_loop()
    b = await loop.run_in_executor(None, fetch, url)
    c = await loop.run_in_executor(None, functools.partial(fetch, url))
    d = await sync_to_async(fetch)(url)
    e = await anyio.to_thread.run_sync(fetch, url)
    f = await shell.offloaded_rev()
    return a, b, c, d, e, f


async def coroutines(url):
    a = await http.afetch(url)
    b = await http.make_coroutine(url)
    return a, b


async def runs_later(url):
    def inner():
        return fetch(url)

    cb = lambda: fetch(url)  # noqa: E731

    class Job:
        result = fetch(url)

    later = http.later(url)
    deferred = http.deferred(url)
    return await asyncio.to_thread(inner), cb, Job, later, deferred


async def lookalikes(url):
    await asyncio.sleep(1)
    client = make_client()
    r = await client.get(url)
    return local_get(url), http.build_url(url, "x"), r


async def trio_nap():
    await trio.sleep(1)


async def unknown(fn, handlers, url):
    a = fn(url)
    b = handlers[url](url)
    return a, b


def sync_view(url):
    return fetch(url)
