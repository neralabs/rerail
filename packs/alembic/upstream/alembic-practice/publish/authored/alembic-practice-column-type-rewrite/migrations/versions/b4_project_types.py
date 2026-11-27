"""types read through the project's TypeDecorators, subclasses, aliases and imports

Revision ID: b4
Revises: b3
"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy import BigInteger as Int64, Integer as Int
from sqlalchemy.dialects import postgresql as pg

from app.types import Email, Int32, MediumText, StringUUID

revision = "b4"
down_revision = "b3"

SmallText = sa.Text


def upgrade():
    # The same PostgreSQL type, spelled through the project.
    op.alter_column("tenants", "uid", existing_type=pg.UUID(), type_=StringUUID())
    op.alter_column("tenants", "seats", existing_type=Int(), type_=Int32())
    op.alter_column("tenants", "note", existing_type=SmallText(), type_=MediumText())
    op.alter_column("tenants", "contact", existing_type=sa.VARCHAR(80), type_=Email(120))
    # Real changes, through the same names.
    op.alter_column("tenants", "quota", existing_type=Int32(), type_=Int64())  # caught
    op.alter_column("tenants", "ref", existing_type=StringUUID(), type_=sa.String(36))  # caught
    op.alter_column("tenants", "bio", existing_type=MediumText(), type_=Email(40))  # caught


def downgrade():
    pass
