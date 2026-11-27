from sqlalchemy import create_engine
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine
from sqlalchemy.orm import scoped_session, sessionmaker

engine = create_engine("sqlite://")
SessionLocal = sessionmaker(bind=engine)
ScopedSession = scoped_session(SessionLocal)
AsyncSessionLocal = async_sessionmaker(create_async_engine("sqlite+aiosqlite://"), expire_on_commit=False)
