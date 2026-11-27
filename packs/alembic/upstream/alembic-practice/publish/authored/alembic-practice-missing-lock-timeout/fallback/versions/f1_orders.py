"""orders: versions outside env.py's directory (version_locations)"""
import sqlalchemy as sa
from alembic import op

revision = "f1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("orders", sa.Column("channel", sa.String(20), nullable=True))


def downgrade():
    op.drop_column("orders", "channel")
