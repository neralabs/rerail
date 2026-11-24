# Not a migration: no revision identifiers, so the same call shape is left alone.
import sqlalchemy as sa
from alembic import op


def add_flag():
    op.add_column("users", sa.Column("flag", sa.Boolean(), nullable=False))
