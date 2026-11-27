"""add t18.c18"""
import sqlalchemy as sa
from alembic import op

revision = "g18"
down_revision = "g17"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t18", sa.Column("c18", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t18", "c18")
