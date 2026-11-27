"""constraints that add_column brings, and constraints on tables made by raw SQL"""
import sqlalchemy as sa
from alembic import op

revision = "c5"
down_revision = "c4"
branch_labels = None

PG_NOT_VALID = {"postgresql_not_valid": True}


def upgrade():
    # Alembic emits a second, validated ALTER TABLE ... ADD FOREIGN KEY / ADD CHECK after the ADD COLUMN.
    op.add_column("payments", sa.Column("user_id", sa.Integer(), sa.ForeignKey("users.id"), nullable=True))  # caught
    op.add_column("payments", sa.Column("n", sa.Integer(), sa.CheckConstraint("n > 0")))  # caught
    with op.batch_alter_table("refunds") as batch_op:
        batch_op.add_column(sa.Column("payment_id", sa.Integer(), sa.ForeignKey("payments.id")))  # caught
    # A foreign key on a column this upgrade just added: inline_references=True is the cheaper fix.
    op.add_column("payments", sa.Column("shop_id", sa.Integer(), nullable=True))
    op.create_foreign_key("fk_payments_shop", "payments", "shops", ["shop_id"], ["id"])  # caught
    op.create_foreign_key(None, source_table="payments", referent_table="taxes", local_cols=["tax_id"], remote_cols=["id"])  # caught

    # The inline reference: PostgreSQL marks it valid at once for a nullable column.
    op.add_column("payments", sa.Column("org_id", sa.Integer(), sa.ForeignKey("orgs.id")), inline_references=True)
    # NOT VALID passed through a dict of dialect keywords.
    op.create_foreign_key("fk_payments_tax", "payments", "taxes", ["tax_id"], ["id"], **PG_NOT_VALID)
    # A table this upgrade creates with raw SQL is empty.
    op.execute("CREATE TABLE audit (id serial primary key, order_id int)")
    op.create_foreign_key("fk_audit_order", "audit", "orders", ["order_id"], ["id"])
    op.execute('CREATE TABLE IF NOT EXISTS "audit_items" (id serial primary key, audit_id int)')
    op.create_foreign_key("fk_audit_items_audit", "audit_items", "audit", ["audit_id"], ["id"])
    # An existing table whose name starts like the raw-SQL one: still reported.
    op.create_check_constraint("ck_audit_log_level", "audit_log", "level > 0")  # caught


def downgrade():
    op.drop_column("payments", "user_id")
