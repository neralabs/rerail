"""a rename only SQLite's branch rebuilds this way"""
import sqlalchemy as sa
from alembic import op

revision = "b9"
down_revision = "b8"
branch_labels = None
depends_on = None


def upgrade():
    if op.get_context().dialect.name == "sqlite":
        op.add_column("notes", sa.Column("body_text", sa.Text()))
        op.drop_column("notes", "body")
    else:
        op.alter_column("notes", "body", new_column_name="body_text")


def downgrade():
    op.alter_column("notes", "body_text", new_column_name="body")
