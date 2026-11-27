"""add t19.c19"""
import sqlalchemy as sa
from alembic import op

revision = "g19"
down_revision = "g18"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t19", sa.Column("c19", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t19", "c19")
