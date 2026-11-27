"""set: sa.text and TO"""
import sqlalchemy as sa
from alembic import op

revision = "s2"
down_revision = "s1"
branch_labels = None
depends_on = None


def upgrade():
    op.execute(sa.text("SET lock_timeout TO '10s'"))
    op.add_column("tasks", sa.Column("s2", sa.Integer(), nullable=True))


def downgrade():
    pass
