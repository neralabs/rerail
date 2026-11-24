"""init"""
import sqlalchemy as sa
from alembic import op

revision = "a1"
down_revision = None
branch_labels = None


def upgrade():
    op.create_table(
        "users",
        sa.Column("id", sa.Integer(), primary_key=True),
        sa.Column("nick", sa.String(40)),
        sa.Column("email", sa.String(255)),
    )
    op.create_table("hero", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("power", sa.Integer()))
    op.create_table("audit_log", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("old_note", sa.Text()))
    op.create_table("widgets", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("size", sa.Integer()))


def downgrade():
    op.drop_table("widgets")
    op.drop_table("audit_log")
    op.drop_table("hero")
    op.drop_table("users")
