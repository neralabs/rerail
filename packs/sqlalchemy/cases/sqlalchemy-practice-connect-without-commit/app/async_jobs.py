from sqlalchemy import insert, text, update

from app.db import async_engine
from app.models import Event, users


async def add_event(kind: str) -> None:
    async with async_engine.connect() as conn:
        await conn.execute(insert(Event).values(kind=kind))  # caught


async def rename(user_id: int, name: str) -> None:
    conn = await async_engine.connect()
    await conn.execute(update(users).where(users.c.id == user_id).values(name=name))  # caught
    await conn.close()


async def add_event_committed(kind: str) -> None:
    async with async_engine.connect() as conn:
        await conn.execute(insert(Event).values(kind=kind))
        await conn.commit()


async def add_event_in_begin(kind: str) -> None:
    async with async_engine.begin() as conn:
        await conn.execute(text("INSERT INTO events (kind) VALUES (:k)"), {"k": kind})
