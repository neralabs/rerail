"""email on u1"""
import sqlalchemy as sa
from alembic import op

revision = "u2"
down_revision = "u1"


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
