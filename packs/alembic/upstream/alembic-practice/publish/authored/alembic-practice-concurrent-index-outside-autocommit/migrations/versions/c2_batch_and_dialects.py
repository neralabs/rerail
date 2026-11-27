"""batch forms, dialect branches, helpers called inside the block"""
from alembic import op

revision = "c2"
down_revision = "c1"
branch_labels = None


def _is_pg(conn):
    return conn.dialect.name == "postgresql"


def upgrade():
    with op.batch_alter_table("orders") as batch_op:
        batch_op.create_index("ix_orders_ref", ["ref"], postgresql_concurrently=True)  # caught
        batch_op.drop_index("ix_orders_old", postgresql_concurrently=True)  # caught

    with op.get_context().autocommit_block():
        with op.batch_alter_table("orders") as batch_op:
            batch_op.create_index("ix_orders_total", ["total"], postgresql_concurrently=True)

    conn = op.get_bind()
    if _is_pg(conn):
        # The PostgreSQL branch still runs in the revision's transaction.
        op.create_index("ix_orders_state", "orders", ["state"], postgresql_concurrently=True)  # caught
    else:
        op.create_index("ix_orders_state", "orders", ["state"], postgresql_concurrently=True)
    if conn.dialect.name == "sqlite":
        op.create_index("ix_orders_note", "orders", ["note"], postgresql_concurrently=True)

    # A helper called inside the block runs outside the transaction too.
    with op.get_context().autocommit_block():
        _paid_index()

    # A MigrationContext held in a local.
    ctx = op.get_context()
    with ctx.autocommit_block():
        op.create_index("ix_orders_user", "orders", ["user_id"], postgresql_concurrently=True)


def _paid_index():
    op.create_index("ix_orders_paid", "orders", ["paid"], postgresql_concurrently=True)


def downgrade():
    _drop_ref()


def _drop_ref():
    op.drop_index("ix_orders_ref", table_name="orders", postgresql_concurrently=True)  # caught
