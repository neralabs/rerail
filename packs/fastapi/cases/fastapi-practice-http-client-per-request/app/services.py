import aiohttp
import httpx


async def fetch_json(url: str):
    async with aiohttp.ClientSession() as session:  # caught
        async with session.get(url) as resp:
            return await resp.json()


async def notify(url: str):
    return await _post(url)


async def _post(url: str):
    async with httpx.AsyncClient() as client:  # caught
        return await client.post(url)


def sync_lookup(url: str):
    with httpx.Client() as client:  # caught
        return client.get(url).json()


def export_all(urls: list[str]):
    with httpx.Client() as client:
        return [client.get(u).json() for u in urls]
