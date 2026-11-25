"""add t23.c23"""
import sqlalchemy as sa
from alembic import op

revision = "g23"
down_revision = "g22"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t23", sa.Column("c23", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t23", "c23")
