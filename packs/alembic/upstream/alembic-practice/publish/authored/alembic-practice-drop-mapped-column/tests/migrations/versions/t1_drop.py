"""a migration in a test tree"""
from alembic import op

revision = "t1"
down_revision = None
branch_labels = None


def upgrade():
    op.drop_column("users", "nick")


def downgrade():
    pass
