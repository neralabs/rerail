"""add missing uuid columns"""
import sqlalchemy as sa
from alembic import op

revision = "n7"
down_revision = "n6"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("slices", sa.Column("uuid", sa.String(36), nullable=True))


def downgrade():
    """This revision repairs b56500de1855, which skipped some uuid columns; there is no downgrade."""
