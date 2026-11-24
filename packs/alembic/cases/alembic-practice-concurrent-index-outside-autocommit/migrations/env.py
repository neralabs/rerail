from alembic import context

from app.db import engine, metadata


def run_migrations_online():
    with engine.connect() as connection:
        # One transaction per revision instead of one for the run: each revision still runs in a transaction.
        context.configure(connection=connection, target_metadata=metadata, transaction_per_migration=True)
        with context.begin_transaction():
            context.run_migrations()


run_migrations_online()
