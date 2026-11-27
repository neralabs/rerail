"""drop columns the models still map"""
import sqlalchemy as sa
from alembic import op

revision = "a2"
down_revision = "a1"
branch_labels = None

USERS = "users"


def upgrade():
    op.drop_column("users", "nick")  # caught
    op.drop_column(USERS, "display_name")  # caught
    op.drop_column(table_name="users", column_name="handle")  # caught
    op.drop_column("users", "bio")  # caught
    op.drop_column("users", "sig")  # caught
    op.drop_column("users", "tz")  # caught
    op.drop_column("users", "display")  # attribute name, not the column's name: not mapped
    op.drop_column("users", "old_avatar")  # no model maps it
    op.drop_column("audit_log", "old_note")  # no model maps the table
    op.drop_column("widgets", "size")  # only a model in a test file maps it
    with op.batch_alter_table("users") as batch_op:
        batch_op.drop_column("is_deleted")  # caught
        batch_op.alter_column("email", new_column_name="email_address")  # caught
        batch_op.alter_column("email", nullable=False)
    op.drop_column("user_tags", "tag_id")  # caught
    op.drop_column("users", "legacy_token")  # the model sets it to None: unmapped
    op.drop_column("users", "api_token")  # caught
    _drop_power()


def _drop_power():
    op.drop_column(op.f("hero"), "power")  # caught


def downgrade():
    op.add_column("users", sa.Column("nick", sa.String(40)))
    op.drop_column("users", "nick")
