"""add t10.c10"""
import sqlalchemy as sa
from alembic import op

revision = "g10"
down_revision = "g09"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t10", sa.Column("c10", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t10", "c10")
