"""copy with op.execute before the drop"""
import sqlalchemy as sa
from alembic import op

revision = "b1"
down_revision = "a4"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("handle", sa.String(40)))
    op.execute("UPDATE users SET handle = nick")
    op.drop_column("users", "nick")


def downgrade():
    op.add_column("users", sa.Column("nick", sa.VARCHAR(length=40)))
    op.execute("UPDATE users SET nick = handle")
    op.drop_column("users", "handle")
