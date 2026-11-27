"""drop and rename tables"""
import sqlalchemy as sa
from alembic import op

revision = "a3"
down_revision = "a2"
branch_labels = None


def upgrade():
    op.drop_table("hero")  # caught
    op.rename_table("orders", "purchases")  # caught
    op.alter_column("users", "nick", new_column_name="nickname")  # caught
    op.drop_table("audit_log")  # no model maps it
    op.drop_table("legacy")  # caught
    op.alter_column("legacy", "code", new_column_name="legacy_code")  # caught
    op.alter_column("legacy", "id", nullable=False)
    op.drop_table("user_tags")  # caught


def downgrade():
    op.rename_table("purchases", "orders")
