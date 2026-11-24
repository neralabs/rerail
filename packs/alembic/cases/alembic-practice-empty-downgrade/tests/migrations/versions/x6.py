"""add t6.c6"""
import sqlalchemy as sa
from alembic import op

revision = "x6"
down_revision = "x5"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t6", sa.Column("c6", sa.Integer(), nullable=True))


def downgrade():
    pass
