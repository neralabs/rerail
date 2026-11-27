"""a migration-shaped fixture under tests"""
import sqlalchemy as sa
from alembic import op

revision = "t1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("handle", sa.String(40)))
    op.drop_column("users", "nick")


def downgrade():
    op.add_column("users", sa.Column("nick", sa.VARCHAR(length=40)))
    op.drop_column("users", "handle")
