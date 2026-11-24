"""labels: lock_timeout in a module-level helper"""
import sqlalchemy as sa
from alembic import op

revision = "c15"
down_revision = "c14"
branch_labels = None
depends_on = None


def _set_lock_timeout(bind):
    if bind.dialect.name == "postgresql":
        bind.exec_driver_sql("SET LOCAL lock_timeout = '3s'")


def upgrade():
    _set_lock_timeout(op.get_bind())
    op.drop_index("ix_labels_old", table_name="labels")


def downgrade():
    op.create_index("ix_labels_old", "labels", ["old"])
