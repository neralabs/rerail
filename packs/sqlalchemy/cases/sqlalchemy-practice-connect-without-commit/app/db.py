from sqlalchemy import create_engine
from sqlalchemy.ext.asyncio import create_async_engine

engine = create_engine("postgresql+psycopg://app@db/app")
async_engine = create_async_engine("postgresql+asyncpg://app@db/app")

# Every statement on these engines' connections commits on its own.
autocommit_engine = create_engine("postgresql+psycopg://app@db/app", isolation_level="AUTOCOMMIT")
maintenance_engine = create_engine("postgresql+psycopg://app@db/app").execution_options(
    isolation_level="AUTOCOMMIT"
)
