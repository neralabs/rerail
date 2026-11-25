"""add users.locale; a TODO is no recorded decision"""
import sqlalchemy as sa
from alembic import op

revision = "c6"
down_revision = "c5"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("locale", sa.String(8), nullable=True))


def downgrade():  # caught
    # TODO: drop the column
    pass


def _unused():
    # FIXME elsewhere in the file does not matter
    return
