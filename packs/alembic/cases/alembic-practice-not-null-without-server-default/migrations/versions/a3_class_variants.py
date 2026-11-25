"""NOT NULL columns spelled through names, aliases and constants

Revision ID: a3
Revises: a2
"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy import Column as Col, Integer as Int

revision = "a3"
down_revision = "a2"
branch_labels = None
depends_on = None

REQUIRED = False
BigInt = sa.BigInteger


def upgrade():
    # A Column bound to a name first.
    tier = sa.Column("tier", sa.String(20), nullable=False)  # caught
    op.add_column("members", tier)
    op.add_column("members", Col("rank", sa.String(8), nullable=REQUIRED))  # caught
    op.add_column("members", sa.Column("uid", sa.Uuid(), primary_key=True))  # caught
    # Integer primary keys, however spelled, become SERIAL and number the existing rows.
    op.add_column("orgs", sa.Column("seq", BigInt, primary_key=True))
    op.add_column("orgs", Col("seq2", Int(), primary_key=True))
    # A table emptied first holds no rows when the column is added.
    op.execute("DELETE FROM sessions")
    op.add_column("sessions", sa.Column("kind", sa.String(20), nullable=False))
    # Only part of a table deleted: it may still hold rows.
    op.execute("DELETE FROM tokens WHERE expired")
    op.add_column("tokens", sa.Column("scope", sa.String(20), nullable=False))  # caught


def downgrade():
    pass
