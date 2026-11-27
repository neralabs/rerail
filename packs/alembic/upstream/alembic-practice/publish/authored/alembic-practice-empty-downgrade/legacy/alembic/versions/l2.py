"""add t2.c2"""
import sqlalchemy as sa
from alembic import op

revision = "l2"
down_revision = "l1"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t2", sa.Column("c2", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t2", "c2")
