"""billing init"""
import sqlalchemy as sa
from alembic import op

revision = "b1"
down_revision = None


def upgrade():
    op.create_table("users", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("nick", sa.String(40)))
    op.create_table("invoices", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("memo", sa.Text()))


def downgrade():
    op.drop_table("invoices")
    op.drop_table("users")
