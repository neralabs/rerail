"""add t17.c17"""
import sqlalchemy as sa
from alembic import op

revision = "g17"
down_revision = "g16"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t17", sa.Column("c17", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t17", "c17")
