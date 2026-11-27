"""users: env.py of this tree sets lock_timeout"""
import sqlalchemy as sa
from alembic import op

revision = "l1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("locale", sa.String(10), nullable=True))
    op.drop_column("users", "legacy")


def downgrade():
    op.drop_column("users", "locale")
