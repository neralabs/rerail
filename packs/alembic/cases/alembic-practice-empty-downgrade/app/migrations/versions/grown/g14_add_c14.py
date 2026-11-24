"""add t14.c14"""
import sqlalchemy as sa
from alembic import op

revision = "g14"
down_revision = "g13"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t14", sa.Column("c14", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t14", "c14")
