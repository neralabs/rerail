"""asyncpg server_settings, the engine re-exported by the package"""
import sqlalchemy as sa
from alembic import op

revision = "e4"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("nick", sa.String(40), nullable=True))


def downgrade():
    pass
