"""a migration the test suite runs against a throwaway database"""
from alembic import op

revision = "t1"
down_revision = None
branch_labels = None


def upgrade():
    op.alter_column("users", "email", nullable=False)


def downgrade():
    op.alter_column("users", "email", nullable=True)
