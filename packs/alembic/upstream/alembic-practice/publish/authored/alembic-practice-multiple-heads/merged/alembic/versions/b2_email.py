"""add users.email"""
import sqlalchemy as sa
from alembic import op

revision = "b2"
down_revision = "b1"
branch_labels = None

def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    pass
