from alembic import context

from autocommit_iso.db import engine


def run_migrations_online():
    with engine.connect() as connection:
        context.configure(connection=connection)
        with context.begin_transaction():
            context.run_migrations()


run_migrations_online()
