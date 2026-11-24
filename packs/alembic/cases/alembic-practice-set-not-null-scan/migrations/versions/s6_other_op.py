"""an `op` that is not alembic.op"""
from myproject import migrate_ops as op

revision = "s6"
down_revision = "s5"
branch_labels = None


def upgrade():
    op.alter_column("users", "email", nullable=False)


def downgrade():
    pass
