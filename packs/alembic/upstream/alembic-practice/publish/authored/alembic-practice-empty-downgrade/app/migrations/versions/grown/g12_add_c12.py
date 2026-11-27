"""add t12.c12"""
import sqlalchemy as sa
from alembic import op

revision = "g12"
down_revision = "g11"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t12", sa.Column("c12", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t12", "c12")
