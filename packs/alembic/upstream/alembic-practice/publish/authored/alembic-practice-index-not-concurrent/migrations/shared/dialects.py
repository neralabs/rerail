from alembic import op


def is_mysql():
    return op.get_bind().dialect.name in {"mysql", "mariadb"}


def is_postgres():
    """True on PostgreSQL."""
    return op.get_bind().dialect.name == "postgresql"


def postgres_enabled(settings):
    # Reads a setting: tells nothing about the database this migration runs on.
    return settings.USE_POSTGRES
