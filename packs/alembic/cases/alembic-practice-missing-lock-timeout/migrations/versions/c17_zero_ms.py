"""projects: '0ms' through SESSION ... TO"""
import sqlalchemy as sa
from alembic import op

revision = "c17"
down_revision = "c16"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SET SESSION lock_timeout TO '0ms'")
    op.create_index("ix_projects_nick", "projects", ["nick"])  # caught


def downgrade():
    pass
