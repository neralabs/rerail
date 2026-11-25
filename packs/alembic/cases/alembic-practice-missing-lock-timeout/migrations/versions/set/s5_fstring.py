"""set: an f-string"""
import sqlalchemy as sa
from alembic import op

revision = "s5"
down_revision = "s4"
branch_labels = None
depends_on = None

LOCK_TIMEOUT = "5s"


def upgrade():
    op.execute(f"SET LOCAL lock_timeout = '{LOCK_TIMEOUT}'")
    op.rename_table("tasks", "jobs")


def downgrade():
    pass
