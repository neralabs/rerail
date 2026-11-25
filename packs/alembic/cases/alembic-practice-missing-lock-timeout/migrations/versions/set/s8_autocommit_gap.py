"""set before an autocommit block: the block runs without it (documented gap, not reported)"""
import sqlalchemy as sa
from alembic import op

revision = "s8"
down_revision = "s7"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SET LOCAL lock_timeout = '5s'")
    with op.get_context().autocommit_block():
        op.add_column("jobs", sa.Column("s8", sa.Integer(), nullable=True))


def downgrade():
    pass
