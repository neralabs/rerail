from alembic import context
from sqlalchemy import engine_from_config, pool, text

config = context.config


def run_migrations_online():
    connectable = engine_from_config(config.get_section(config.config_ini_section), poolclass=pool.NullPool)
    with connectable.connect() as connection:
        context.configure(connection=connection, target_metadata=None, transaction_per_migration=True)
        with context.begin_transaction():
            connection.execute(text("SET lock_timeout = '10s'"))
            context.run_migrations()


run_migrations_online()
