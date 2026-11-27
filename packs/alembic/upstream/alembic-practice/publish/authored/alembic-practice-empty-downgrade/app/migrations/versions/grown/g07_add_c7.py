"""add t7.c7"""
import sqlalchemy as sa
from alembic import op

revision = "g07"
down_revision = "g06"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t7", sa.Column("c7", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t7", "c7")
