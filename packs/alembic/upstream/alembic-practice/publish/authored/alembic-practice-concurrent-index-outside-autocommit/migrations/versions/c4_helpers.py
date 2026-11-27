"""helpers: called inside the block, and one also called outside it (polar's shape, then a slip)"""
from alembic import op

revision = "c4"
down_revision = "c3"
branch_labels = None

TABLE = "benefit_grants"


def _drop_index(name):
    op.drop_index(name, table_name=TABLE, if_exists=True, postgresql_concurrently=True)


def _create_index(name, columns):
    try:
        op.create_index(name, TABLE, columns, unique=True, postgresql_concurrently=True)
    except Exception:
        _drop_index(name)
        raise


def _rebuild(name, columns):
    op.create_index(name, TABLE, columns, postgresql_concurrently=True)  # caught


def upgrade():
    op.execute("SET LOCAL lock_timeout = '5s'")
    with op.get_context().autocommit_block():
        _create_index("ix_grants_scope", ["customer_id", "benefit_id"])
        _drop_index("ix_grants_legacy")
        _rebuild("ix_grants_member", ["member_id"])
    # Called again after the block: this call runs in the transaction.
    _rebuild("ix_grants_order", ["order_id"])


def downgrade():
    with op.get_context().autocommit_block():
        _drop_index("ix_grants_scope")
