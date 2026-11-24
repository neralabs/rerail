"""orders.user_id foreign key"""
import sqlalchemy as sa
from alembic import op

revision = "c4"
down_revision = "c3"
branch_labels = None
depends_on = None


def upgrade():
    op.create_foreign_key("fk_orders_user", "orders", "users", ["user_id"], ["id"])


def downgrade():  # caught
    pass
    ...
