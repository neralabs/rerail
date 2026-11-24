from collections.abc import AsyncIterator, Iterator

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
