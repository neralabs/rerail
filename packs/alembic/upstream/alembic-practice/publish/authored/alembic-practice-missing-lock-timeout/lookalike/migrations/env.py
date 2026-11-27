from alembic import context
from app.redis_db import REDIS_LOCK, engine

config = context.config


def run_migrations_online():
    with engine.connect() as connection:
        context.configure(connection=connection, target_metadata=None)  # caught
        with context.begin_transaction():
            context.run_migrations()


run_migrations_online()
