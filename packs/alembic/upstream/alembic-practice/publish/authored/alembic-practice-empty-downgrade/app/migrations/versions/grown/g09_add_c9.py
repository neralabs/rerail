"""add t9.c9"""
import sqlalchemy as sa
from alembic import op

revision = "g09"
down_revision = "g08"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t9", sa.Column("c9", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t9", "c9")
