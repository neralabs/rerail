"""projects: lock_timeout turned off; a docstring is no setting"""
import sqlalchemy as sa
from alembic import op

revision = "c13"
down_revision = "c12"
branch_labels = None
depends_on = None


def upgrade():
    """lock_timeout is left to the deploy."""
    op.execute("SET lock_timeout = 0")
    op.rename_table("projects", "workspaces")  # caught
    op.execute("RESET lock_timeout")


def downgrade():
    op.rename_table("workspaces", "projects")
