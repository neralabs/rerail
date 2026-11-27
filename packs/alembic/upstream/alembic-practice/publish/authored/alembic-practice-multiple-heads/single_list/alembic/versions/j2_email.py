"""email on j1, its parent a list of one, through a constant"""
import sqlalchemy as sa
from alembic import op

BASE = "j1"

revision = "j2"
down_revision = [BASE]  # caught


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
