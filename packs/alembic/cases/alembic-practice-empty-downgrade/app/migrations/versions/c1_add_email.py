"""add users.email"""
import sqlalchemy as sa
from alembic import op

revision = "c1"
down_revision = "g16"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255), nullable=True))


def downgrade():  # caught
    pass
