import asyncio

from alembic import context

from app import async_engine

config = context.config


def do_run_migrations(connection):
    context.configure(connection=connection, target_metadata=None)
    with context.begin_transaction():
        context.run_migrations()


async def run_migrations_online():
    async with async_engine.connect() as connection:
        await connection.run_sync(do_run_migrations)


asyncio.run(run_migrations_online())
