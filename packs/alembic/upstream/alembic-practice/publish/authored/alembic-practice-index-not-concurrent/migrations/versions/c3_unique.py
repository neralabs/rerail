"""unique constraints"""
import sqlalchemy as sa
from alembic import op

revision = "c3"
down_revision = "c2"
branch_labels = None


def upgrade():
    op.create_unique_constraint("uq_users_email", "users", ["email"])  # caught
    op.create_unique_constraint(op.f("uq_accounts_slug"), table_name="accounts", columns=["slug"])  # caught
    with op.batch_alter_table("orders") as batch_op:
        batch_op.create_unique_constraint("uq_orders_ref", ["ref"])  # caught

    # A new table: nothing to lock.
    op.create_table("plans", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("code", sa.String(20)))
    op.create_unique_constraint("uq_plans_code", "plans", ["code"])

    # The concurrent path: a unique index built concurrently, then promoted to a constraint.
    with op.get_context().autocommit_block():
        op.drop_index("uq_users_handle", table_name="users", if_exists=True, postgresql_concurrently=True)
        op.create_index("uq_users_handle", "users", ["handle"], unique=True, postgresql_concurrently=True)
    op.execute("ALTER TABLE users ADD CONSTRAINT uq_users_handle UNIQUE USING INDEX uq_users_handle")

    # Other constraints are other rules' concern.
    op.create_foreign_key("fk_orders_user", "orders", "users", ["user_id"], ["id"])
    op.create_check_constraint("ck_orders_total", "orders", "total >= 0")


def downgrade():
    op.drop_constraint("uq_users_email", "users", type_="unique")
    op.create_unique_constraint("uq_users_legacy", "users", ["legacy"])
