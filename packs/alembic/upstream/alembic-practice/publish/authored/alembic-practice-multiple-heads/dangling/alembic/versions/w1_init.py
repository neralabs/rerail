"""a base beside a line whose history is not in the run: left alone"""
import sqlalchemy as sa
from alembic import op

revision = "w1"
down_revision = None


def upgrade():
    op.create_table("users", sa.Column("id", sa.Integer(), primary_key=True))


def downgrade():
    op.drop_table("users")
