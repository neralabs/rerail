"""init"""
import sqlalchemy as sa
from alembic import op

revision = "a1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("users", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("users")
