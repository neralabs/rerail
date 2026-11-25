"""order rows on k1: the head of the second base's line"""
import sqlalchemy as sa
from alembic import op

revision = "k2"
down_revision = "k1"  # caught


def upgrade():
    op.create_table("order_rows", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("order_rows")
