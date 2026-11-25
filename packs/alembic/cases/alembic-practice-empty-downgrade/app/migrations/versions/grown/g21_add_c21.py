"""add t21.c21"""
import sqlalchemy as sa
from alembic import op

revision = "g21"
down_revision = "g20"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t21", sa.Column("c21", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t21", "c21")
