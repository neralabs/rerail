"""calls named update/insert that move no rows, and types through aliases"""
import sqlalchemy as sa
from alembic import op
from sqlalchemy import String as Str

revision = "a6"
down_revision = "a5"
branch_labels = None


def insert(items, value):
    items.insert(0, value)


def upgrade():
    options = {"echo": False}
    options.update(echo=True)
    order = []
    insert(order, "labels")
    op.add_column("labels", sa.Column("caption", Str(length=60), nullable=True))
    op.drop_column("labels", "title")  # caught


def downgrade():
    op.add_column("labels", sa.Column("title", sa.VARCHAR(length=60), nullable=True))
    op.drop_column("labels", "caption")
