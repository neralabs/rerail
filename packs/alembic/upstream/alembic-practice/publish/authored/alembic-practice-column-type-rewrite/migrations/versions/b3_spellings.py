"""the same PostgreSQL type spelled two ways, next to real changes

Revision ID: b3
Revises: b2
"""
import sqlalchemy as sa
import sqlmodel
from alembic import op
from sqlalchemy.dialects import postgresql

revision = "b3"
down_revision = "b2"


def upgrade():
    # The same type on both sides: autogenerate restates it next to a nullability change.
    op.alter_column("events", "at", existing_type=sa.TIMESTAMP(), type_=sa.DateTime())
    op.alter_column("events", "uid", existing_type=postgresql.UUID(), type_=sa.Uuid())
    op.alter_column("events", "n", existing_type=sa.INTEGER(), type_=sa.Integer(), nullable=True)
    op.alter_column("events", "name", existing_type=sa.VARCHAR(length=255), type_=sqlmodel.sql.sqltypes.AutoString(length=255))
    op.alter_column("events", "ok", existing_type=sa.BOOLEAN(), type_=sa.Boolean())
    op.alter_column("events", "body", existing_type=sa.TEXT(), type_=sa.Text())
    op.alter_column("events", "seen", existing_type=postgresql.TIMESTAMP(timezone=True), type_=sa.DateTime(timezone=True))
    # Real changes.
    op.alter_column("events", "big", existing_type=sa.INTEGER(), type_=sa.BigInteger())  # caught
    op.alter_column("events", "at2", existing_type=sa.DateTime(), type_=sa.DateTime(timezone=True))  # caught
    op.alter_column("events", "short", existing_type=sa.VARCHAR(length=255), type_=sqlmodel.sql.sqltypes.AutoString(length=80))  # caught
    op.alter_column("events", "tags", existing_type=postgresql.ARRAY(sa.VARCHAR(50)), type_=postgresql.ARRAY(sa.String(100)))  # caught


def downgrade():
    pass
