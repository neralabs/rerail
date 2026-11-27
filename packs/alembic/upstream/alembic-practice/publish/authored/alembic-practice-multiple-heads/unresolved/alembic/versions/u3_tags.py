"""parent imported from another module: unknown, so the line is not read as a second base"""
import sqlalchemy as sa
from alembic import op

from .ids import PREV

revision = "u3"
down_revision = PREV


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("tags")
