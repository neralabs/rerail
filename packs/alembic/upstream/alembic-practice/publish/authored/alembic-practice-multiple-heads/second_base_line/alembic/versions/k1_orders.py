"""orders: a second base, not a head itself"""
import sqlalchemy as sa
from alembic import op

revision = "k1"
down_revision = None


def upgrade():
    op.create_table("orders", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("orders")
