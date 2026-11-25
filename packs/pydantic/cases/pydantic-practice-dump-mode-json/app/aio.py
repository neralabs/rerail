from datetime import datetime

import aiohttp
import orjson
from aiohttp import ClientSession, web

from app.lookalike import ClientSession as LocalSession
from app.schemas import Event, Invoice, Plain


# ---- reported: aiohttp encodes json= with json.dumps unless the session has its own json_serialize
async def push(e: Event, url: str) -> None:
    async with aiohttp.ClientSession() as s:
        await s.post(url, json=e.model_dump())  # caught


async def push_assigned(invoice: Invoice, url: str) -> None:
    session = ClientSession()
    async with session.put(url, json={"invoice": invoice.model_dump()}) as resp:  # caught
        resp.raise_for_status()
    await session.close()


async def one_shot(e: Event, url: str) -> None:
    async with aiohttp.request("POST", url, json=e.model_dump()) as resp:  # caught
        resp.raise_for_status()


async def handler(request: web.Request) -> web.Response:
    event = Event(name="ping", starts_at=datetime.now())
    return web.json_response(event.model_dump())  # caught


class Notifier:
    def __init__(self) -> None:
        self._session: aiohttp.ClientSession | None = None

    async def start(self) -> None:
        self._session = aiohttp.ClientSession()

    async def notify(self, e: Event) -> None:
        await self._session.post("/notify", json=e.model_dump())  # caught


# ---- left alone
async def own_serializer(e: Event, url: str) -> None:
    async with aiohttp.ClientSession(json_serialize=lambda o: orjson.dumps(o).decode()) as s:
        await s.post(url, json=e.model_dump())


async def configured_session(e: Event, url: str, **options) -> None:
    async with aiohttp.ClientSession(**options) as s:
        await s.post(url, json=e.model_dump())


class OrjsonNotifier:
    def __init__(self) -> None:
        self.session = aiohttp.ClientSession(json_serialize=lambda o: orjson.dumps(o).decode())

    async def notify(self, e: Event) -> None:
        await self.session.post("/notify", json=e.model_dump())


async def own_dumps(request: web.Request) -> web.Response:
    event = Event(name="ping", starts_at=datetime.now())
    return web.json_response(event.model_dump(), dumps=lambda o: orjson.dumps(o).decode())


def lookalike_session(e: Event, url: str) -> None:
    LocalSession().post(url, json=e.model_dump())


async def json_native(plain: Plain, url: str) -> None:
    async with aiohttp.ClientSession() as s:
        await s.post(url, json=plain.model_dump())
