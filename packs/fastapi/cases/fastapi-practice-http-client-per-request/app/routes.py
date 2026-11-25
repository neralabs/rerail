import aiohttp
import httpx
from aiohttp import ClientSession
from fastapi import APIRouter, BackgroundTasks, Depends, Request
from socketio import AsyncClient

from .clients import (
    Svc,
    Weather,
    geo,
    get_async_client,
    get_billing,
    get_client,
    get_session,
    get_shared_http,
    get_svc,
    lazy,
    make_client,
)
from .deps import Upstream, get_http
from .services import fetch_json, notify, sync_lookup
from .worker import send_webhook

router = APIRouter()


@router.get("/a")
async def proxy_a(url: str):
    async with httpx.AsyncClient() as client:  # caught
        return (await client.get(url)).json()


@router.get("/b")
def proxy_b(url: str):
    client = httpx.Client(timeout=3)  # caught
    try:
        return client.get(url).json()
    finally:
        client.close()


@router.get("/c")
async def proxy_c(url: str):
    session = aiohttp.ClientSession()  # caught
    try:
        async with session.get(url) as resp:
            return await resp.json()
    finally:
        await session.close()


@router.get("/d")
async def proxy_d(url: str):
    async with ClientSession(trust_env=True) as session:  # caught
        async with session.get(url) as resp:
            return await resp.text()


@router.get("/e")
async def via_helper(url: str):
    return await fetch_json(url)


@router.get("/e2")
async def via_helper_again(url: str):
    return await fetch_json(url)


@router.post("/f")
async def via_two_helpers(url: str):
    return await notify(url)


@router.get("/g")
def via_sync_helper(url: str):
    return sync_lookup(url)


@router.get("/h")
async def via_factory(url: str):
    async with make_client() as client:
        return (await client.get(url)).json()


@router.get("/i")
def via_class():
    return Weather("https://weather.example.com").today()


@router.get("/dep")
async def via_dep(http: httpx.AsyncClient = Depends(get_http), up: dict = Depends(Upstream("https://up.example.com"))):
    return (await http.get("https://example.com")).json(), up


# Near misses: a shared client, a cached factory, a lazy global, a singleton, background work, other libraries.
@router.get("/ok/state")
async def ok_state(request: Request, url: str):
    return (await request.app.state.http.get(url)).json()


@router.get("/ok/cached")
def ok_cached(url: str):
    return get_client().get(url).json()


@router.get("/ok/cached-async")
async def ok_cached_async(url: str):
    return (await get_async_client().get(url)).json()


@router.get("/ok/pool")
async def ok_pool(url: str):
    session = await get_session()
    async with session.get(url) as resp:
        return await resp.json()


@router.get("/ok/singleton")
def ok_singleton(ip: str):
    return geo.lookup(ip)


@router.get("/ok/lazy")
async def ok_lazy(url: str):
    client = await lazy.client()
    return (await client.get(url)).json()


@router.post("/ok/background")
async def ok_background(url: str, tasks: BackgroundTasks):
    tasks.add_task(send_webhook, url, {"ok": True})
    return {"queued": True}


@router.get("/ok/socketio")
async def ok_socketio():
    sio = AsyncClient()
    await sio.connect("https://socket.example.com")
    return {"ok": True}


@router.get("/ok/transport")
async def ok_transport(url: str):
    transport = httpx.AsyncHTTPTransport(local_address="0.0.0.0")
    async with httpx.AsyncClient(transport=transport) as client:
        return (await client.get(url)).json()


@router.get("/ok/nested")
async def ok_nested(url: str):
    async def later():
        async with httpx.AsyncClient() as client:
            return await client.get(url)

    return {"later": later}


@router.get("/ok/billing")
async def ok_billing():
    return (await get_billing().http.get("https://billing.example.com/plans")).json()


# The module-level helpers build a client per call.
@router.get("/j")
def per_call_get(url: str):
    return httpx.get(url, timeout=5).json()  # caught


@router.get("/k")
async def per_call_aiohttp(url: str):
    async with aiohttp.request("GET", url) as resp:  # caught
        return await resp.json()


# One client per websocket connection, reused for the socket's lifetime: reported (log mode absorbs it).
@router.websocket("/ws")
async def relay(websocket):
    async with httpx.AsyncClient() as client:  # caught
        while True:
            url = await websocket.receive_text()
            await websocket.send_text((await client.get(url)).text)


# A cached dependency builds its service, and the service's client, once: the shared-client idiom.
@router.get("/ok/cached-dep")
async def ok_cached_dep(svc: Svc = Depends(get_svc), http: httpx.AsyncClient = Depends(get_shared_http)):
    return {"ok": True}
