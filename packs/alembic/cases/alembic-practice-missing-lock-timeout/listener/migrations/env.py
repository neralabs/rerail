from alembic import context
from app.listener_db import engine

config = context.config


def run_migrations_online():
    with engine.connect() as connection:
        context.configure(connection=connection, target_metadata=None)
        with context.begin_transaction():
            context.run_migrations()


run_migrations_online()
