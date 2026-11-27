from alembic import context
from sqlalchemy import engine_from_config, pool, text

config = context.config


def run_migrations_online():
    connectable = engine_from_config(config.get_section(config.config_ini_section), poolclass=pool.NullPool)
    with connectable.connect() as connection:
        connection.execute(text("SET lock_timeout = '5s'"))
        context.configure(connection=connection, target_metadata=None)
        with context.begin_transaction():
            context.run_migrations()


run_migrations_online()
