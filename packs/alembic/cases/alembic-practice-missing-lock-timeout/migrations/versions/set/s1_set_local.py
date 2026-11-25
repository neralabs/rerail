"""set: SET LOCAL"""
import sqlalchemy as sa
from alembic import op

revision = "s1"
down_revision = "c20"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SET LOCAL lock_timeout = '5s'")
    op.add_column("tasks", sa.Column("s1", sa.Integer(), nullable=True))


def downgrade():
    pass
