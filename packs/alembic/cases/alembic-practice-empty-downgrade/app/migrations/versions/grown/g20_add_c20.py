"""add t20.c20"""
import sqlalchemy as sa
from alembic import op

revision = "g20"
down_revision = "g19"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t20", sa.Column("c20", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t20", "c20")
