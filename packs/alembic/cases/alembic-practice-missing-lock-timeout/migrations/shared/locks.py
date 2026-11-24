from alembic import op

LOCK_TIMEOUT = "SET LOCAL lock_timeout = '5s'"


def set_lock_timeout():
    op.execute("SET LOCAL lock_timeout = '5s'")


def set_statement_timeout():
    op.execute("SET LOCAL statement_timeout = '60s'")
