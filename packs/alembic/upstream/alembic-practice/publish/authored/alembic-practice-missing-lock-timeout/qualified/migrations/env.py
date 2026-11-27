import alembic.context
from sqlalchemy import engine_from_config, pool

config = alembic.context.config


def run_migrations_online():
    connectable = engine_from_config(config.get_section(config.config_ini_section), poolclass=pool.NullPool)
    with connectable.connect() as connection:
        alembic.context.configure(connection=connection, target_metadata=None)  # caught
        with alembic.context.begin_transaction():
            alembic.context.run_migrations()


run_migrations_online()
