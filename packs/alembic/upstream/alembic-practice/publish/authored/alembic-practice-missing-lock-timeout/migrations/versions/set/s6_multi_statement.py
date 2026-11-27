"""set: one of several statements"""
import sqlalchemy as sa
from alembic import op

revision = "s6"
down_revision = "s5"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SET LOCAL statement_timeout = '60s'; SET LOCAL lock_timeout = '2s'")
    op.create_foreign_key("fk_jobs_owner", "jobs", "users", ["owner_id"], ["id"])


def downgrade():
    pass
