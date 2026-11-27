from alembic import context
from sqlalchemy import create_engine


def run_migrations_online():
    engine = create_engine("postgresql+psycopg://localhost/app")
    with engine.connect() as connection:
        # No transaction around the revisions: CONCURRENTLY runs as is.
        context.configure(connection=connection, transactional_ddl=False)
        with context.begin_transaction():
            context.run_migrations()


run_migrations_online()
