"""Runs the migrations. lock_timeout is not set here."""
import logging

from alembic import context
from sqlalchemy import engine_from_config, pool

config = context.config
log = logging.getLogger(__name__)
# TODO: lock_timeout


def run_migrations_offline():
    context.configure(url=config.get_main_option("sqlalchemy.url"), literal_binds=True)
    with context.begin_transaction():
        context.run_migrations()


def run_migrations_online():
    log.info("remember to set lock_timeout one day")
    connectable = engine_from_config(config.get_section(config.config_ini_section), poolclass=pool.NullPool)
    with connectable.connect() as connection:
        context.configure(connection=connection, target_metadata=None)  # caught
        with context.begin_transaction():
            context.run_migrations()


if context.is_offline_mode():
    run_migrations_offline()
else:
    run_migrations_online()
