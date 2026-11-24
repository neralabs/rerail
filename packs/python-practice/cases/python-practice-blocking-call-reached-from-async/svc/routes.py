import asyncio
import time

import requests
from fastapi import FastAPI

from svc.services import http, shell
from svc.services.http import fetch, retry_fetch
from svc.services.loops import refresh_now, refresh_via_asgiref, refresh_with_loop

app = FastAPI()


@app.get("/page")
async def page(url: str):
    r = fetch(url)  # caught
    data = http.post_json(url, {"a": 1})  # caught
    body = http.download(url)  # caught
    ip = http.resolve("example.com")  # caught
    again = retry_fetch(url)  # caught
    return r, data, body, ip, again


@app.get("/rev")
async def rev():
    return shell.git_rev()  # caught


class Worker:
    def __init__(self, path):
        self.path = path

    def sweep(self):
        shell.flush_disks()

    async def run(self):
        self.sweep()  # caught


async def reload():
    refresh_now()  # caught
    refresh_with_loop()  # caught
    refresh_via_asgiref()  # caught


async def direct_loop(coro, other):
    asyncio.run(coro)  # caught
    asyncio.get_event_loop().run_until_complete(other)  # caught


async def direct_blocking(url):
    # Written in the async def itself: the linter's own async rules report these.
    time.sleep(1)
    return requests.get(url, timeout=5)
