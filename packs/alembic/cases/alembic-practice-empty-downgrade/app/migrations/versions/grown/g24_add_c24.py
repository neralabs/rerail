"""add t24.c24"""
import sqlalchemy as sa
from alembic import op

revision = "g24"
down_revision = "g23"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t24", sa.Column("c24", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t24", "c24")
