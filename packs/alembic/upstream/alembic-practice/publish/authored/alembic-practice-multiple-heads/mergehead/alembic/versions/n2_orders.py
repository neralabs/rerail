"""orders"""
import sqlalchemy as sa
from alembic import op

revision = "n2"
down_revision = "n1"
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("orders", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    pass
