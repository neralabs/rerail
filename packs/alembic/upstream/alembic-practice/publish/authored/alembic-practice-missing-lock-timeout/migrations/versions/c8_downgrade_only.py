"""sessions: only data in upgrade, lock_timeout only in downgrade"""
import sqlalchemy as sa
from alembic import op

revision = "c8"
down_revision = "c7"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("DELETE FROM sessions WHERE expires_at < now()")


def downgrade():
    op.execute("SET LOCAL lock_timeout = '5s'")
    op.drop_column("sessions", "device")
