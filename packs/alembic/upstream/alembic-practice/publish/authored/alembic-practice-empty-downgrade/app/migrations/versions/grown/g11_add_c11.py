"""add t11.c11"""
import sqlalchemy as sa
from alembic import op

revision = "g11"
down_revision = "g10"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t11", sa.Column("c11", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t11", "c11")
