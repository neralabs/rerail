import functools

import aiohttp
import httpx

_session: aiohttp.ClientSession | None = None


async def get_session() -> aiohttp.ClientSession:
    global _session
    if _session is None or _session.closed:
        _session = aiohttp.ClientSession(trust_env=True)
    return _session


@functools.lru_cache
def get_client() -> httpx.Client:
    return httpx.Client(timeout=5)


@functools.cache
def get_async_client() -> httpx.AsyncClient:
    return httpx.AsyncClient(timeout=5)


def make_client() -> httpx.AsyncClient:
    return httpx.AsyncClient(timeout=5)  # caught


class Weather:
    def __init__(self, base_url: str):
        self.client = httpx.Client(base_url=base_url)  # caught

    def today(self):
        return self.client.get("/today").json()


class Geo:
    def __init__(self, base_url: str):
        self.client = httpx.Client(base_url=base_url)

    def lookup(self, ip: str):
        return self.client.get(f"/ip/{ip}").json()


class Lazy:
    def __init__(self):
        self._client = None

    async def client(self) -> httpx.AsyncClient:
        if self._client is None:
            self._client = httpx.AsyncClient()
        return self._client


geo = Geo("https://geo.example.com")
lazy = Lazy()


class Billing:
    def __init__(self, token: str):
        self.http = httpx.AsyncClient(headers={"Authorization": token})


_billing: Billing | None = None


def get_billing() -> Billing:
    global _billing
    if _billing is None:
        _billing = Billing("secret")
    return _billing
