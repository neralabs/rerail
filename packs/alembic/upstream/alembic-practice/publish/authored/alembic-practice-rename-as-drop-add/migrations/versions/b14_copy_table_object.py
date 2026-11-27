"""a copy through a lightweight table object's update()"""
import sqlalchemy as sa
from alembic import op

revision = "b14"
down_revision = "b13"
branch_labels = None

notes = sa.table("notes", sa.column("body", sa.Text()), sa.column("text", sa.Text()))


def upgrade():
    op.add_column("notes", sa.Column("body", sa.Text(), nullable=True))
    op.execute(notes.update().values(body=notes.c.text))
    op.drop_column("notes", "text")


def downgrade():
    op.add_column("notes", sa.Column("text", sa.TEXT(), nullable=True))
    op.drop_column("notes", "body")
