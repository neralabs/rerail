"""a test fixture revision"""
import sqlalchemy as sa
from alembic import op

revision = "t1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("x", sa.Integer()))


def downgrade():
    op.drop_column("users", "x")
