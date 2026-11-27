"""payments: lock_timeout set after the first lock"""
import sqlalchemy as sa
from alembic import op

revision = "c4"
down_revision = "c3"
branch_labels = None
depends_on = None


def upgrade():
    op.create_index("ix_payments_state", "payments", ["state"])  # caught
    op.execute("SET LOCAL lock_timeout = '5s'")
    op.add_column("payments", sa.Column("settled_at", sa.DateTime(), nullable=True))


def downgrade():
    op.drop_column("payments", "settled_at")
    op.drop_index("ix_payments_state", table_name="payments")
