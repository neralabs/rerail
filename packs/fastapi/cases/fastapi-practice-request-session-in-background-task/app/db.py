from collections.abc import AsyncIterator, Iterator

from fastapi import Request
from sqlalchemy import create_engine
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker, create_async_engine
from sqlalchemy.orm import Session, sessionmaker

SessionLocal = sessionmaker(bind=create_engine("postgresql://localhost/app"))
AsyncSessionLocal = async_sessionmaker(bind=create_async_engine("postgresql+asyncpg://localhost/app"))


def get_db() -> Iterator[Session]:
    with SessionLocal() as db:
        yield db
        db.commit()


async def get_async_db() -> AsyncIterator[AsyncSession]:
    async with AsyncSessionLocal() as db:
        yield db


# The middleware's session (app/main.py), handed to routes by a dependency.
def get_state_db(request: Request) -> Session:
    session = request.state.db
    return session


# A yield dependency that puts its own session on the request state: open until the background tasks finish.
def get_tx(request: Request) -> Iterator[Session]:
    with SessionLocal() as db:
        request.state.tx = db
        yield db
