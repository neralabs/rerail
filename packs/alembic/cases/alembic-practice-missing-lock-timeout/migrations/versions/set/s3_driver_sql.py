"""set: SESSION through the driver"""
import sqlalchemy as sa
from alembic import op

revision = "s3"
down_revision = "s2"
branch_labels = None
depends_on = None


def upgrade():
    op.get_bind().exec_driver_sql("set session lock_timeout = 3000")
    op.drop_column("tasks", "s1")


def downgrade():
    pass
