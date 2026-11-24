"""teams: lock_timeout through a shared constant"""
import sqlalchemy as sa
from alembic import op

from migrations.shared import locks

revision = "c11"
down_revision = "c10"
branch_labels = None
depends_on = None


def upgrade():
    op.execute(locks.LOCK_TIMEOUT)
    op.rename_table("team_members", "memberships")


def downgrade():
    op.rename_table("memberships", "team_members")
