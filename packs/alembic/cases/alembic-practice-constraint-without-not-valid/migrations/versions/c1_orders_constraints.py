"""constraints on the existing orders table"""
import sqlalchemy as sa
from alembic import op

revision = "c1"
down_revision = "c0"
branch_labels = None

ORDERS = "orders"


def upgrade():
    op.add_column("orders", sa.Column("user_id", sa.Integer(), nullable=True))
    op.create_foreign_key("fk_orders_user", "orders", "users", ["user_id"], ["id"])  # caught
    op.create_foreign_key(  # caught
        constraint_name="fk_orders_shop",
        source_table="orders",
        referent_table="shops",
        local_cols=["shop_id"],
        remote_cols=["id"],
        ondelete="CASCADE",
    )
    op.create_foreign_key(op.f("fk_orders_coupon"), ORDERS, "coupons", ["coupon_id"], ["id"])  # caught
    op.create_foreign_key(None, "orders", "carts", ["cart_id"], ["id"], postgresql_not_valid=False)  # caught
    op.create_check_constraint("ck_orders_total", "orders", "total >= 0")  # caught
    op.create_check_constraint("ck_orders_qty", table_name="orders", condition=sa.text("qty > 0"))  # caught

    # The safe form: added NOT VALID, validated in a later revision (c3).
    op.create_foreign_key("fk_orders_agent", "orders", "agents", ["agent_id"], ["id"], postgresql_not_valid=True)
    op.create_check_constraint("ck_orders_paid", "orders", "paid >= 0", postgresql_not_valid=True)
    # Not known to be off: left alone.
    op.create_foreign_key("fk_orders_store", "orders", "stores", ["store_id"], ["id"], postgresql_not_valid=NOT_VALID)


NOT_VALID = True


def downgrade():
    # A rollback re-adds the old constraint: left alone.
    op.create_foreign_key("fk_orders_legacy", "orders", "legacy", ["legacy_id"], ["id"])
    op.drop_constraint("fk_orders_user", "orders", type_="foreignkey")
