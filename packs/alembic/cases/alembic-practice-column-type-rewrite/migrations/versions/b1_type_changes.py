"""type changes on existing tables

Revision ID: b1
Revises: b0
"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects import mysql, postgresql

from app.types import LongText, MediumText

revision = "b1"
down_revision = "b0"
branch_labels = None
depends_on = None


def upgrade():
    # Rewrites: a different storage type.
    op.alter_column("workflow_runs", "total_tokens", existing_type=sa.INTEGER(), type_=sa.BigInteger())  # caught
    op.alter_column("documents", "meta", existing_type=postgresql.JSON(astext_type=sa.Text()), type_=postgresql.JSONB(astext_type=sa.Text()))  # caught
    op.alter_column("ingredients", "quantity", existing_type=sa.Integer(), type_=sa.Float())  # caught
    op.alter_column("costs", "amount", existing_type=sa.Integer(), type_=sa.Numeric())  # caught
    op.alter_column("entity", "source", existing_type=sa.BOOLEAN(), type_=sa.String(), existing_nullable=True)  # caught
    op.alter_column("prices", "amount", existing_type=sa.Numeric(10, 2), type_=sa.Numeric(12, 4))  # caught
    op.alter_column("prices", "net", existing_type=sa.Numeric(10, 2), type_=sa.Numeric(8, 2))  # caught
    op.alter_column(  # caught
        table_name="file",
        column_name="meta",
        existing_type=sa.Text(),
        type_=sa.JSON(),
        postgresql_using="meta::json",
    )
    with op.batch_alter_table("folder", schema=None) as batch_op:
        batch_op.alter_column(  # caught
            "created_at",
            type_=sa.BigInteger(),
            existing_type=sa.DateTime(),
            postgresql_using="extract(epoch from created_at)::bigint",
        )
        batch_op.alter_column("updated_at", server_default=None)
    # Length limits: PostgreSQL checks every row.
    op.alter_column("tools", "name", existing_type=sa.VARCHAR(length=256), type_=sa.String(length=255))  # caught
    op.alter_column("user", "email", existing_type=sa.String(), type_=sa.String(length=255))  # caught
    op.alter_column("bindings", "ext_id", existing_type=sa.TEXT(), type_=sa.String(512))  # caught
    op.alter_column("tools", "label", existing_type=sa.Text, type_=sa.VARCHAR(40))  # caught
    # No existing_type, and the target cannot be a widening.
    op.alter_column("accounts", "balance", type_=sa.BigInteger())  # caught

    # Safe widenings: PostgreSQL changes the catalog only.
    op.alter_column("providers", "model", existing_type=sa.VARCHAR(length=40), type_=sa.String(length=255))
    op.alter_column("report", "crontab", existing_type=sa.VARCHAR(50), type_=sa.VARCHAR(length=1000))
    op.alter_column("report", "key", existing_type=sa.String(64), type_=sa.String(64))
    op.alter_column("report", "note", existing_type=sa.String(64), type_=sa.String())
    op.alter_column("report", "body", existing_type=sa.String(length=1000), type_=sa.Text())
    op.alter_column("report", "title", existing_type=sa.VARCHAR(length=255), type_=sa.UnicodeText())
    op.alter_column("report", "json", existing_type=sa.Text(), type_=sa.String())
    op.alter_column("report", "blob", existing_type=sa.LargeBinary(), type_=sa.LargeBinary(length=2**24 - 1))
    op.alter_column("prices", "gross", existing_type=sa.Numeric(10, 2), type_=sa.Numeric(14, 2))
    op.alter_column("prices", "tax", existing_type=sa.Numeric(10, 2), type_=sa.Numeric())
    op.alter_column("report", "same", existing_type=sa.DateTime(), type_=sa.DateTime())
    # Project types that are TEXT on PostgreSQL and MEDIUMTEXT/LONGTEXT on MySQL.
    op.alter_column("dashboards", "position_json", existing_type=sa.Text(), type_=MediumText())
    op.alter_column("apps", "disclaimer", existing_type=sa.VARCHAR(length=255), type_=LongText())
    # A MySQL-only target.
    op.alter_column("table_schema", "description", existing_type=sa.Text, type_=mysql.LONGTEXT)
    # A length that is not a literal.
    current_length = 64
    op.alter_column("ab_user", "username", existing_type=sa.String(current_length), type_=sa.String(128))
    op.alter_column("ab_user", "nick", existing_type=sa.String(64), type_=sa.String(current_length))
    op.alter_column(
        "ab_user",
        "first_name",
        existing_type=sa.String(current_length) if current_length else sa.String(64),
        type_=sa.String(32),
    )
    # No existing_type, and the target may be a widening.
    op.alter_column("accounts", "label", type_=sa.Text())
    op.alter_column("accounts", "code", type_=sa.String(255))
    # Not a type change.
    op.alter_column("accounts", "name", existing_type=sa.String(40), nullable=False)
    op.alter_column("accounts", "kind", existing_type=sa.String(40), new_column_name="category")

    # A table the same upgrade() creates is empty.
    op.create_table("events", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("n", sa.Integer()))
    op.alter_column("events", "n", existing_type=sa.Integer(), type_=sa.BigInteger())
    with op.batch_alter_table("events") as batch_op:
        batch_op.alter_column("n", existing_type=sa.Integer(), type_=sa.Float())


def downgrade():
    op.alter_column("workflow_runs", "total_tokens", existing_type=sa.BigInteger(), type_=sa.INTEGER())
