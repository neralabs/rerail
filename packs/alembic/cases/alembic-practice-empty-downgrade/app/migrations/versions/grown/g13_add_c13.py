"""add t13.c13"""
import sqlalchemy as sa
from alembic import op

revision = "g13"
down_revision = "g12"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("t13", sa.Column("c13", sa.Integer(), nullable=True))


def downgrade():
    op.drop_column("t13", "c13")
