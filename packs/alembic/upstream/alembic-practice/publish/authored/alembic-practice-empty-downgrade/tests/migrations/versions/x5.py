"""add t5.c5"""
import sqlalchemy as sa
from alembic import op

revision = "x5"
down_revision = "x4"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t5", sa.Column("c5", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t5", "c5")
