"""an intentional second base: it carries a branch label (`alembic upgrade plugins@head`)"""
import sqlalchemy as sa
from alembic import op

revision = "h1"
down_revision = None
branch_labels = ("plugins",)


def upgrade():
    op.create_table("plugins", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("plugins")
