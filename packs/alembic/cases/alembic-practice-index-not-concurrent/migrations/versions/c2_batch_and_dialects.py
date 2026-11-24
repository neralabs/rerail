"""batch indexes and dialect branches"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects.mysql.base import MySQLDialect
from sqlalchemy.dialects.sqlite.base import SQLiteDialect

revision = "c2"
down_revision = "c1"
branch_labels = None


def _is_pg(conn):
    return conn.dialect.name == "postgresql"


def upgrade():
    with op.batch_alter_table("orders", schema=None) as batch_op:
        batch_op.add_column(sa.Column("ref", sa.String(40), nullable=True))
        batch_op.create_index("ix_orders_ref", ["ref"], unique=False)  # caught
        batch_op.create_index(batch_op.f("ix_orders_total"), ["total"])  # caught

    # Created in a dialect branch: still a table this upgrade() creates.
    conn = op.get_bind()
    if conn.dialect.name == "postgresql":
        op.create_table("audit", sa.Column("id", sa.Uuid(), primary_key=True), sa.Column("at", sa.DateTime()))
    else:
        op.create_table("audit", sa.Column("id", sa.String(36), primary_key=True), sa.Column("at", sa.DateTime()))
    with op.batch_alter_table("audit") as batch_op:
        batch_op.create_index("ix_audit_at", ["at"])
    op.create_index("ix_audit_id_at", "audit", ["id", "at"])

    # Branches that run only on another database: CONCURRENTLY means nothing there.
    if _is_pg(conn):
        with op.get_context().autocommit_block():
            op.create_index("ix_orders_state", "orders", ["state"], postgresql_concurrently=True)
    else:
        op.create_index("ix_orders_state", "orders", ["state"])
    if conn.dialect.name == "sqlite":
        op.create_index("ix_orders_note", "orders", ["note"])
    if conn.dialect.name != "postgresql":
        op.create_index("ix_orders_paid", "orders", ["paid"])
    # Dialect classes in isinstance tests (superset's): the else of `not SQLite` runs on SQLite only.
    if not isinstance(conn.dialect, SQLiteDialect):
        op.create_unique_constraint("uq_orders_code", "orders", ["code"])  # caught
    else:
        with op.batch_alter_table("orders") as batch_op:
            batch_op.create_unique_constraint("uq_orders_code", ["code"])
    if isinstance(conn.dialect, MySQLDialect):
        op.create_index("ix_orders_code_prefix", "orders", ["code"], mysql_length={"code": 10})


def _index_on(table_name):
    # The table is a parameter: not known here.
    op.create_index(f"ix_{table_name}_owner", table_name, ["owner_id"])


def downgrade():
    with op.batch_alter_table("orders") as batch_op:
        batch_op.create_index("ix_orders_old", ["old"])
