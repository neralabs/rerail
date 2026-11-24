"""Not Alembic: a module with its own `op` and upgrade()/downgrade()."""
from app import ops as op

revision = "x"
down_revision = "w"


def upgrade():
    op.add_column("users", "handle")
    op.drop_column("users", "nick")


def downgrade():
    pass
