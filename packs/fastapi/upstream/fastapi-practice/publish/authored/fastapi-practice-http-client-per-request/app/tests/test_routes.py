import httpx
from fastapi import FastAPI

app = FastAPI()


@app.get("/t")
async def t_route():
    async with httpx.AsyncClient() as client:
        return (await client.get("https://example.com")).json()


async def test_routes():
    async with httpx.AsyncClient(transport=httpx.ASGITransport(app=app), base_url="http://t") as client:
        assert (await client.get("/t")).status_code == 200
