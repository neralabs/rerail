"""add t22.c22"""
import sqlalchemy as sa
from alembic import op

revision = "g22"
down_revision = "g21"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t22", sa.Column("c22", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t22", "c22")
