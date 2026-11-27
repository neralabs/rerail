"""add t16.c16"""
import sqlalchemy as sa
from alembic import op

revision = "g16"
down_revision = "g15"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t16", sa.Column("c16", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t16", "c16")
