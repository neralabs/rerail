"""mixed: two do not"""
import sqlalchemy as sa
from alembic import op

revision = "x3"
down_revision = 'x2'
branch_labels = None
depends_on = None


def upgrade():
    op.drop_column("orders", "x1")


def downgrade():
    pass
