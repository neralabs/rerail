"""email on t1"""
import sqlalchemy as sa
from alembic import op

revision = "t2"
down_revision = "t1"


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
