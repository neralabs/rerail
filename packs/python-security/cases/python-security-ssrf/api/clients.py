import httpx


async def get_client():
    async with httpx.AsyncClient() as client:
        yield client


def get_cache():
    return {}


async def call(client: httpx.AsyncClient, endpoint: str):
    r = await client.post(endpoint, json={"ping": True})  # caught
    return r.json()
