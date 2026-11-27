"""not a migration: no revision/down_revision"""
from alembic import op


def tighten():
    op.alter_column("users", "email", nullable=False)
