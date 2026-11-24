"""teams: lock_timeout through a shared helper"""
import sqlalchemy as sa
from alembic import op

from migrations.shared.locks import set_lock_timeout

revision = "c10"
down_revision = "c9"
branch_labels = None
depends_on = None


def upgrade():
    set_lock_timeout()
    op.add_column("teams", sa.Column("slug", sa.String(50), nullable=True))


def downgrade():
    op.drop_column("teams", "slug")
