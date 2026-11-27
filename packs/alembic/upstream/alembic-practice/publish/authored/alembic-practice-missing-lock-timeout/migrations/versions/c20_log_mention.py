"""projects: a log call that mentions lock_timeout sets nothing"""
import sqlalchemy as sa
from alembic import op
import logging

revision = "c20"
down_revision = "c19"
branch_labels = None
depends_on = None

log = logging.getLogger(__name__)


def upgrade():
    log.info("set lock_timeout before the deploy")
    op.create_unique_constraint("uq_projects_name", "projects", ["name"])  # caught


def downgrade():
    pass
