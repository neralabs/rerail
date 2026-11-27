"""add t6.c6"""
import sqlalchemy as sa
from alembic import op

revision = "g06"
down_revision = "g05"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t6", sa.Column("c6", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t6", "c6")
