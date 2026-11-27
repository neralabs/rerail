"""add t1.c1"""
import sqlalchemy as sa
from alembic import op

revision = "s1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t1", sa.Column("c1", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t1", "c1")
