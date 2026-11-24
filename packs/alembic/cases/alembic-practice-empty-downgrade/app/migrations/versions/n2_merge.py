"""merge heads"""
import sqlalchemy as sa
from alembic import op

revision = "n2"
down_revision = ("n1", "g16")
branch_labels = None
depends_on = None


def upgrade():
    op.create_index("ix_users_email", "users", ["email"])


def downgrade():
    pass
