"""email on a1: the head of the first line"""
import sqlalchemy as sa
from alembic import op

revision = "a2"
down_revision = "a1"  # caught
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
