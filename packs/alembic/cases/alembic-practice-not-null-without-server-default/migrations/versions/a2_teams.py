"""teams: a new table, a backfilled column, and the dialect branches

Revision ID: a2
Revises: a1
"""
from alembic import op
from sqlalchemy import Column, Integer, String, Text

import models

revision = "a2"
down_revision = "a1"
branch_labels = None
depends_on = None


def _is_pg(conn):
    return conn.dialect.name == "postgresql"


def _add_owner():
    op.add_column("projects", Column("owner_id", Integer, nullable=False))  # caught


def upgrade():
    conn = op.get_bind()
    # The table is new, so it has no rows yet.
    op.create_table("teams", Column("id", Integer, primary_key=True))
    op.add_column("teams", Column("name", String(80), nullable=False))
    with op.batch_alter_table("teams") as batch_op:
        batch_op.add_column(Column("slug", String(80), nullable=False))

    # Nullable first, backfill, then NOT NULL.
    op.add_column("projects", Column("team_id", Integer(), nullable=True))
    op.execute("UPDATE projects SET team_id = 1")
    op.alter_column("projects", "team_id", nullable=False)

    _add_owner()

    if _is_pg(conn):
        with op.batch_alter_table("workflows", schema=None) as batch_op:
            batch_op.add_column(Column("variables", Text(), server_default="{}", nullable=False))
            batch_op.add_column(Column("graph", Text(), nullable=False))  # caught
    else:
        with op.batch_alter_table("workflows", schema=None) as batch_op:
            batch_op.add_column(Column("variables", models.types.LongText(), default="{}", nullable=False))
            batch_op.add_column(Column("graph", models.types.LongText(), nullable=False))

    if conn.dialect.name == "sqlite":
        op.add_column("projects", Column("label", String(20), nullable=False))


def downgrade():
    op.drop_table("teams")
