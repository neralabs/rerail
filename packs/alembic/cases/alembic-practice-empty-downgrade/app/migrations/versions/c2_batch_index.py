"""index orders.created_at"""
import sqlalchemy as sa
from alembic import op

revision: str = "c2"
down_revision: str | None = "c1"
branch_labels = None
depends_on = None


def upgrade() -> None:
    with op.batch_alter_table("orders") as batch_op:
        batch_op.create_index("ix_orders_created_at", ["created_at"])


def downgrade() -> None:  # caught
    ...
