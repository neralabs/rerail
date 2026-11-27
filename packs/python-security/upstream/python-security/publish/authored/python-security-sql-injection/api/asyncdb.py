import asyncpg
import databases
from fastapi import APIRouter

router = APIRouter()
database = databases.Database("postgresql://localhost/app")


@router.get("/people")
async def people(name: str, conn):
    return await conn.fetch(f"SELECT * FROM people WHERE name = '{name}'")  # caught


@router.get("/people/one")
async def person(name: str):
    pool = await asyncpg.create_pool("postgresql://localhost/app")
    async with pool.acquire() as handle:
        row = await handle.fetchrow(f"SELECT * FROM people WHERE name = '{name}'")  # caught
        row = await handle.fetchrow("SELECT * FROM people WHERE name = $1", name)
    return row


@router.get("/people/count")
async def count(name: str, link: asyncpg.Connection):
    return await link.fetchval("SELECT count(*) FROM people WHERE name = '" + name + "'")  # caught


@router.get("/people/all")
async def all_people(name: str):
    rows = await database.fetch_all(f"SELECT * FROM people WHERE name = '{name}'")  # caught
    rows += await database.fetch_all("SELECT * FROM people WHERE name = :name", {"name": name})
    return rows


@router.get("/cache")
async def cache(key: str, client):
    # An HTTP or cache client's `fetch` is no database call.
    return await client.fetch(f"/items/{key}")
