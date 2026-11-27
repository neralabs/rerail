"""add t8.c8"""
import sqlalchemy as sa
from alembic import op

revision = "g08"
down_revision = "g07"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t8", sa.Column("c8", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t8", "c8")
