"""events: concurrent index builds only"""
import sqlalchemy as sa
from alembic import op

revision = "c6"
down_revision = "c5"
branch_labels = None
depends_on = None


def upgrade():
    with op.get_context().autocommit_block():
        op.drop_index("ix_events_old", table_name="events", if_exists=True, postgresql_concurrently=True)
        op.create_index("ix_events_at", "events", ["at"], postgresql_concurrently=True)


def downgrade():
    op.drop_index("ix_events_at", table_name="events")
