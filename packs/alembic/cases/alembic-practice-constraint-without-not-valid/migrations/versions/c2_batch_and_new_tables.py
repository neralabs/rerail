"""batch forms, tables created in the same upgrade(), dialect branches"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects.sqlite.base import SQLiteDialect

revision = "c2"
down_revision = "c1"
branch_labels = None


def upgrade():
    with op.batch_alter_table("orders", schema=None) as batch_op:
        batch_op.add_column(sa.Column("invoice_id", sa.Integer(), nullable=True))
        batch_op.create_foreign_key("fk_orders_invoice", "invoices", ["invoice_id"], ["id"])  # caught
        batch_op.create_check_constraint("ck_orders_invoice", "invoice_id > 0")  # caught
        batch_op.create_foreign_key(
            "fk_orders_ledger", "ledgers", ["ledger_id"], ["id"], postgresql_not_valid=True
        )

    # A new table: empty, so validation is free. Constraints inside create_table are no operation of this kind.
    op.create_table(
        "invoices",
        sa.Column("id", sa.Integer(), primary_key=True),
        sa.Column("order_id", sa.Integer(), sa.ForeignKey("orders.id")),
        sa.Column("amount", sa.Integer()),
        sa.CheckConstraint("amount >= 0", name="ck_invoices_amount"),
    )
    op.create_foreign_key("fk_invoices_user", "invoices", "users", ["user_id"], ["id"])
    op.create_check_constraint("ck_invoices_positive", "invoices", "amount > 0")
    with op.batch_alter_table("invoices") as batch_op:
        batch_op.create_foreign_key("fk_invoices_shop", "shops", ["shop_id"], ["id"])
    # An existing table pointing at the new one still scans the existing rows.
    op.create_foreign_key("fk_users_invoice", "users", "invoices", ["last_invoice_id"], ["id"])  # caught

    # Branches that run only on another database: NOT VALID means nothing there.
    conn = op.get_bind()
    if conn.dialect.name == "postgresql":
        op.create_foreign_key("fk_users_shop", "users", "shops", ["shop_id"], ["id"], postgresql_not_valid=True)
    else:
        op.create_foreign_key("fk_users_shop", "users", "shops", ["shop_id"], ["id"])
    if conn.dialect.name == "sqlite":
        op.create_check_constraint("ck_users_age", "users", "age >= 0")
    if not isinstance(conn.dialect, SQLiteDialect):
        op.create_check_constraint("ck_users_score", "users", "score >= 0")  # caught
    else:
        with op.batch_alter_table("users") as batch_op:
            batch_op.create_check_constraint("ck_users_score", "score >= 0")
    _link_users()


def _link_users():
    # A module-level helper upgrade() calls: runs as part of it.
    op.create_foreign_key("fk_users_team", "users", "teams", ["team_id"], ["id"])  # caught


def _fk_on(table_name):
    # The table is a parameter: not known here.
    op.create_foreign_key(f"fk_{table_name}_owner", table_name, "users", ["owner_id"], ["id"])


def downgrade():
    with op.batch_alter_table("orders") as batch_op:
        batch_op.create_foreign_key("fk_orders_old", "old", ["old_id"], ["id"])
