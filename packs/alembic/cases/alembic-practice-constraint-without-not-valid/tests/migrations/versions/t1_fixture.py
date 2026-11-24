"""a migration the test suite runs against a throwaway database"""
from alembic import op

revision = "t1"
down_revision = None
branch_labels = None


def upgrade():
    op.create_foreign_key("fk_orders_user", "orders", "users", ["user_id"], ["id"])
    op.create_check_constraint("ck_orders_total", "orders", "total >= 0")


def downgrade():
    op.drop_constraint("fk_orders_user", "orders", type_="foreignkey")
