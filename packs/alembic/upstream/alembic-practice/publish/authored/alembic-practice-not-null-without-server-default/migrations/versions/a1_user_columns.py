"""add columns to users and accounts

Revision ID: a1
Revises: a0
"""
import sqlalchemy as sa
from alembic import op

revision = "a1"
down_revision = "a0"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("nick", sa.String(40), nullable=False))  # caught
    op.add_column("users", sa.Column("bio", sa.Text(), nullable=False, default="x"))  # caught
    op.add_column("users", sa.Column("prefs", sa.JSON(), default=sa.text("'{}'"), nullable=False))  # caught
    op.add_column(table_name="users", column=sa.Column("kind", sa.String(20), nullable=False))  # caught
    op.add_column(
        "users",
        sa.Column(  # caught
            "status",
            sa.String(20),
            nullable=False,
        ),
    )
    op.add_column("users", sa.Column("code", sa.String(8), primary_key=True))  # caught
    op.add_column("users", sa.Column("seq", sa.Integer(), primary_key=True, autoincrement=False))  # caught
    op.add_column("users", sa.Column("score", sa.Integer(), server_default=None, nullable=False))  # caught
    with op.batch_alter_table("accounts", schema=None) as batch_op:
        batch_op.add_column(sa.Column("plan", sa.String(20), nullable=False))  # caught
        batch_op.add_column(sa.Column("tier", sa.String(20), default="free", nullable=False))  # caught
        batch_op.add_column(sa.Column("seats", sa.Integer(), server_default="1", nullable=False))

    # A server default fills the existing rows.
    op.add_column("users", sa.Column("active", sa.Boolean(), server_default=sa.false(), nullable=False))
    op.add_column("users", sa.Column("title", sa.String(40), server_default="", nullable=False))
    op.add_column("users", sa.Column("created", sa.DateTime(), server_default=sa.text("now()"), nullable=False))
    op.add_column("users", sa.Column("level", sa.Integer(), nullable=False, server_default="0", default=0))
    # Nullable columns need no value.
    op.add_column("users", sa.Column("nickname", sa.String(40)))
    op.add_column("users", sa.Column("alias", sa.String(40), nullable=True))
    # Generated columns compute their own value.
    op.add_column("users", sa.Column("total", sa.Integer(), sa.Computed("a + b"), nullable=False))
    op.add_column("users", sa.Column("rowid", sa.BigInteger(), sa.Identity(), nullable=False))
    # An integer primary key becomes SERIAL on PostgreSQL, which numbers the existing rows.
    op.add_column("users", sa.Column("id2", sa.Integer(), primary_key=True))
    op.add_column("users", sa.Column("id3", sa.BigInteger, primary_key=True))


def downgrade():
    op.add_column("users", sa.Column("legacy", sa.String(40), nullable=False))
    op.drop_column("users", "nick")
