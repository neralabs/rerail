"""email on g1"""
import sqlalchemy as sa
from alembic import op

revision = "g2"
down_revision = "g1"
branch_labels = None


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
