import aiohttp
import httpx
import urllib3
from fastapi import APIRouter, Depends, Request
from pydantic import BaseModel, HttpUrl

from api import clients
from api.config import settings

router = APIRouter()
pool = urllib3.PoolManager()


class LoadRequest(BaseModel):
    url: HttpUrl
    name: str


# ---- reported ----------------------------------------------------------------------------------------------------


@router.get("/preview")
async def preview(url: HttpUrl):
    async with httpx.AsyncClient(follow_redirects=True) as client:
        r = await client.get(str(url))  # caught
    return {"status": r.status_code}


@router.post("/load")
async def load(body: LoadRequest):
    async with aiohttp.ClientSession() as session:
        async with session.get(str(body.url)) as resp:  # caught
            return {"text": await resp.text()}


@router.post("/stream")
async def stream(request: Request):
    data = await request.json()
    with httpx.stream("GET", data["source"]) as resp:  # caught
        return {"size": sum(len(c) for c in resp.iter_bytes())}


@router.get("/raw")
def raw(target: str):
    return {"data": pool.request("GET", target).data}  # caught


@router.get("/module")
def module_level(u: str):
    return {"status": httpx.get(u).status_code}  # caught


@router.get("/via-helper")
async def via_helper(endpoint: str, client: httpx.AsyncClient = Depends(clients.get_client)):
    return await clients.call(client, endpoint)


@router.get("/proxy")
async def proxy(request: Request):
    target = request.query_params["target"]
    async with aiohttp.ClientSession() as session:
        async with session.request("GET", url=target) as resp:  # caught
            return {"status": resp.status}


# ---- left alone --------------------------------------------------------------------------------------------------


@router.get("/repos/{owner}/{repo}")
async def repo(owner: str, repo: str):
    async with httpx.AsyncClient() as client:
        r = await client.get(f"https://api.github.com/repos/{owner}/{repo}")
    return r.json()


@router.get("/items/{item_id}")
def item(item_id: int):
    return httpx.get(f"{settings.API_URL}/items/{item_id}").json()


@router.get("/search")
async def search(q: str):
    async with httpx.AsyncClient(base_url=settings.API_URL) as client:
        r = await client.get("/search", params={"q": q})
    return r.json()


@router.post("/named")
async def named(body: LoadRequest):
    return httpx.post(settings.HOOK_URL, json={"name": body.name}).json()


@router.get("/cached")
def cached(key: str, cache=Depends(clients.get_cache)):
    return {"value": cache.get(key)}
