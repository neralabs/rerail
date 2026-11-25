"""email on s1"""
import sqlalchemy as sa
from alembic import op

revision = "s2"
down_revision = "s1"  # caught


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
