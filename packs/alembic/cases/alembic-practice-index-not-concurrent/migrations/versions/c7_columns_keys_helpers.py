"""indexes that add_column, create_primary_key and project helpers build"""
import sqlalchemy as sa
from alembic import op

from migrations.shared.utils import create_index, create_index_concurrently

revision = "c7"
down_revision = "c6"
branch_labels = None


def upgrade():
    # Alembic emits ALTER TABLE ... ADD UNIQUE and CREATE INDEX after the ADD COLUMN.
    op.add_column("accounts", sa.Column("code", sa.String(8), unique=True))  # caught
    op.add_column("accounts", sa.Column("ref", sa.String(8), index=True))  # caught
    with op.batch_alter_table("invoices") as batch_op:
        batch_op.add_column(sa.Column("number", sa.String(20), nullable=True, unique=True))  # caught
    op.add_column("accounts", sa.Column("note", sa.String(80), unique=False, index=False))
    # A primary key on an existing table builds a unique index under ACCESS EXCLUSIVE.
    op.create_primary_key("item_pkey", "item", ["id"])  # caught
    # Project helpers that call op.create_index with the table they are given.
    create_index("purge_audit_log", "ix_purge_audit_log_ts", ["ts"])  # caught
    create_index(table_name="events", index_name="ix_events_kind", columns=["kind"])  # caught
    _idx("ix_payments_state", "payments", ["state"])  # caught
    create_index_concurrently("payments", "ix_payments_ref", ["ref"])

    # A new table: its columns' indexes and key lock nothing.
    op.create_table("ledgers", sa.Column("id", sa.Integer()))
    op.add_column("ledgers", sa.Column("code", sa.String(8), unique=True))
    op.create_primary_key("ledgers_pkey", "ledgers", ["id"])
    create_index("ledgers", "ix_ledgers_code", ["code"])


def _idx(name, table, cols):
    op.create_index(name, table, cols)


def downgrade():
    op.drop_constraint("item_pkey", "item", type_="primary")
