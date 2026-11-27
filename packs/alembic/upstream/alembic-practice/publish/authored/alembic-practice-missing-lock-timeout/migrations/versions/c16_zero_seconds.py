"""projects: '0s' turns lock_timeout off"""
import sqlalchemy as sa
from alembic import op

revision = "c16"
down_revision = "c15"
branch_labels = None
depends_on = None


def upgrade():
    op.execute("SET LOCAL lock_timeout = '0s'")
    op.add_column("projects", sa.Column("nick", sa.String(40), nullable=True))  # caught


def downgrade():
    pass
