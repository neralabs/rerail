"""add t4.c4"""
import sqlalchemy as sa
from alembic import op

revision = "g04"
down_revision = "g03"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t4", sa.Column("c4", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t4", "c4")
