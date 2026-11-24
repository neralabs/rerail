"""add t3.c3"""
import sqlalchemy as sa
from alembic import op

revision = "g03"
down_revision = "g02"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t3", sa.Column("c3", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t3", "c3")
