"""a copy whose SQL is a module constant"""
import sqlalchemy as sa
from alembic import op

revision = "b13"
down_revision = "b12"
branch_labels = None

COPY_TITLE = "UPDATE pages SET heading = title"


def upgrade():
    op.add_column("pages", sa.Column("heading", sa.String(length=120), nullable=True))
    op.execute(COPY_TITLE)
    op.drop_column("pages", "title")


def downgrade():
    op.add_column("pages", sa.Column("title", sa.VARCHAR(length=120), nullable=True))
    op.drop_column("pages", "heading")
