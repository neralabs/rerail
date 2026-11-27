"""make columns of existing tables NOT NULL"""
import sqlalchemy as sa
from alembic import op

revision = "s1"
down_revision = None
branch_labels = None
depends_on = None

ACCOUNTS = "accounts"


def upgrade():
    op.alter_column("users", "email", nullable=False)  # caught
    op.alter_column("users", "name", existing_type=sa.String(64), nullable=False)  # caught
    op.alter_column(table_name="orders", column_name="total", nullable=False, server_default="0")  # caught
    op.alter_column(ACCOUNTS, op.f("owner_id"), existing_type=sa.Integer(), nullable=False)  # caught
    op.alter_column(  # caught
        "users",
        "created_at",
        existing_type=sa.DateTime(),
        nullable=False,
        existing_server_default=sa.text("now()"),
    )
    with op.batch_alter_table("projects") as batch_op:
        batch_op.alter_column("slug", existing_type=sa.String(), nullable=False)  # caught
        batch_op.alter_column(column_name="title", nullable=False)  # caught
    _tighten_teams()

    # Near misses: no SET NOT NULL.
    op.alter_column("users", "bio", existing_type=sa.Text(), nullable=True)
    op.alter_column("users", "age", existing_type=sa.Integer(), existing_nullable=False, type_=sa.BigInteger())
    op.alter_column("users", "nick", server_default="anon")
    # Already NOT NULL: PostgreSQL does nothing.
    op.alter_column("users", "zip", existing_type=sa.String(10), existing_nullable=False, nullable=False)


def _tighten_teams():
    op.alter_column("teams", "plan", nullable=False)  # caught


def downgrade():
    op.alter_column("users", "bio", nullable=False)
    op.alter_column("users", "email", nullable=True)
