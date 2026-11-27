"""rebuild: what an upgrade drops it adds back"""
import sqlalchemy as sa
from alembic import op

revision = "a4"
down_revision = "a3"
branch_labels = None


def upgrade():
    # Swap a column for one of a new type: the mapped name exists again at the end.
    op.add_column("users", sa.Column("new_locale", sa.Text()))
    op.drop_column("users", "locale")
    op.alter_column("users", "new_locale", new_column_name="locale")
    # Drop and re-add.
    op.drop_column("users", "avatar_url")
    op.add_column("users", sa.Column("avatar_url", sa.String(80)))
    with op.batch_alter_table("orders") as batch_op:
        batch_op.drop_column("status")
        batch_op.add_column(sa.Column("status", sa.String(20)))
    # Rebuild a table under the same name.
    op.rename_table("tags", "tags_old")
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True))
    op.drop_table("tags_old")
    # Added before it is dropped: the column is gone at the end.
    op.add_column("hero", sa.Column("secret_name", sa.Text()))
    op.drop_column("hero", "secret_name")  # caught


def downgrade():
    pass
