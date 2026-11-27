"""orders on d1"""
import sqlalchemy as sa
from alembic import op

revision = "d2"
down_revision = "d1"  # caught
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("orders", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("orders")
