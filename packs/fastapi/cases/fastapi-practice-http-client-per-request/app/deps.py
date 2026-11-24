from collections.abc import AsyncIterator

import httpx


async def get_http() -> AsyncIterator[httpx.AsyncClient]:
    async with httpx.AsyncClient(timeout=5) as client:  # caught
        yield client


class Upstream:
    def __init__(self, base: str):
        self.base = base

    async def __call__(self) -> dict:
        async with httpx.AsyncClient(base_url=self.base) as client:  # caught
            return (await client.get("/health")).json()
