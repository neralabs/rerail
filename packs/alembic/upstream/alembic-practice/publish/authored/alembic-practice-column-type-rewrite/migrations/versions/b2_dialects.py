"""dialect branches, fallbacks and a helper

Revision ID: b2
Revises: b1
"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects import mysql, postgresql
from sqlalchemy.dialects.mysql.base import MySQLDialect

revision = "b2"
down_revision = "b1"
branch_labels = None
depends_on = None


def _is_pg(conn):
    return conn.dialect.name == "postgresql"


def _widen_quota():
    op.alter_column("providers", "quota_limit", existing_type=sa.INTEGER(), type_=sa.BigInteger())  # caught


def upgrade():
    conn = op.get_bind()
    _widen_quota()
    if _is_pg(conn):
        with op.batch_alter_table("datasource_providers", schema=None) as batch_op:
            batch_op.alter_column("provider", existing_type=sa.VARCHAR(length=255), type_=sa.String(length=128))  # caught
        with op.batch_alter_table("strategies", schema=None) as batch_op:
            batch_op.alter_column(  # caught
                "exclude_plugins",
                existing_type=postgresql.ARRAY(sa.VARCHAR(length=255)),
                type_=sa.JSON(),
                postgresql_using="to_jsonb(exclude_plugins)::json",
            )
    else:
        with op.batch_alter_table("workflows", schema=None) as batch_op:
            batch_op.alter_column("updated_at", existing_type=mysql.TIMESTAMP(), type_=sa.DateTime())

    if isinstance(conn.dialect, MySQLDialect):
        with op.batch_alter_table("query") as batch_op:
            batch_op.alter_column("sql", existing_type=sa.Text, type_=sa.Integer())

    with op.batch_alter_table("dbs") as batch_op:
        try:
            batch_op.alter_column(  # caught
                "encrypted_extra",
                existing_type=sa.Text(),
                type_=sa.LargeBinary(),
                postgresql_using="encrypted_extra::bytea",
            )
        except TypeError:
            # non-Postgres fallback
            batch_op.alter_column("encrypted_extra", existing_type=sa.Text(), type_=sa.LargeBinary())


def downgrade():
    pass
