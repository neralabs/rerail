"""email on r1, its parent a tuple of one: no merge revision"""
import sqlalchemy as sa
from alembic import op

revision = "r2"
down_revision = ("r1",)  # caught


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
