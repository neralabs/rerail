from alembic import context
from sqlalchemy import create_engine

config = context.config


def run_migrations_online():
    engine = create_engine(
        config.get_main_option("sqlalchemy.url"),
        connect_args={"options": "-c lock_timeout=5000"},
    )
    with engine.connect() as connection:
        context.configure(connection=connection, target_metadata=None)
        with context.begin_transaction():
            context.run_migrations()


run_migrations_online()
