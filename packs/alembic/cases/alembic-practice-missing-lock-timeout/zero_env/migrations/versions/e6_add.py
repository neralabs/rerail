"""the engine's options set lock_timeout=0, which is off"""
import sqlalchemy as sa
from alembic import op

revision = "e6"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("nick", sa.String(40), nullable=True))


def downgrade():
    pass
