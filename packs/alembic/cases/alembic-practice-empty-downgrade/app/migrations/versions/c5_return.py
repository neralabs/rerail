"""add users.phone; a bare return is no downgrade"""
import sqlalchemy as sa
from alembic import op

revision = "c5"
down_revision = "c4"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("phone", sa.String(40), nullable=True))


def downgrade():  # caught
    return
