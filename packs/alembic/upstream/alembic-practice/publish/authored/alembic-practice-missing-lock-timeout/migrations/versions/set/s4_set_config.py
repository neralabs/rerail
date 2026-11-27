"""set: set_config"""
import sqlalchemy as sa
from alembic import op

revision = "s4"
down_revision = "s3"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SELECT set_config('lock_timeout', '5s', true)")
    op.create_index("ix_tasks_s2", "tasks", ["s2"])


def downgrade():
    pass
