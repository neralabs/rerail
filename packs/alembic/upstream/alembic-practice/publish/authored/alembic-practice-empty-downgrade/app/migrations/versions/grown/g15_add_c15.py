"""add t15.c15"""
import sqlalchemy as sa
from alembic import op

revision = "g15"
down_revision = "g14"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t15", sa.Column("c15", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t15", "c15")
