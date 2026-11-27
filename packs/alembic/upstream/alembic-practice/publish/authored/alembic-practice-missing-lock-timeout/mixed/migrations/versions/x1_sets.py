"""mixed: one revision sets it"""
import sqlalchemy as sa
from alembic import op

revision = "x1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SET LOCAL lock_timeout = '5s'")
    op.add_column("orders", sa.Column("x1", sa.Integer(), nullable=True))


def downgrade():
    pass
