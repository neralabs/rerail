"""an `op` that is not alembic.op"""
from myproject import migrate_ops as op

revision = "c4"
down_revision = "c3"
branch_labels = None


def upgrade():
    op.create_foreign_key("fk_orders_user", "orders", "users", ["user_id"], ["id"])
    op.create_check_constraint("ck_orders_total", "orders", "total >= 0")


def downgrade():
    pass
