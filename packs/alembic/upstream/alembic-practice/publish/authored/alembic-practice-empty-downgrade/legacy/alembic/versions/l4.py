"""add t4.c4"""
import sqlalchemy as sa
from alembic import op

revision = "l4"
down_revision = "l3"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t4", sa.Column("c4", sa.Integer(), nullable=True))


def downgrade():
    pass
