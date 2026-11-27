"""add users.email: the fork point"""
import sqlalchemy as sa
from alembic import op

revision = "a2"
down_revision = "a1"
branch_labels = None
depends_on = None

def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    pass
