"""rename tool_api_providers.description_str (batch form, drop first)"""
import sqlalchemy as sa
from alembic import op

revision = "a2"
down_revision = "a1"
branch_labels = None
depends_on = None


def upgrade():
    with op.batch_alter_table("tool_api_providers", schema=None) as batch_op:
        batch_op.drop_column("description_str")  # caught
        batch_op.add_column(sa.Column("description", sa.Text(), nullable=True))


def downgrade():
    with op.batch_alter_table("tool_api_providers", schema=None) as batch_op:
        batch_op.add_column(sa.Column("description_str", sa.TEXT(), autoincrement=False, nullable=True))
        batch_op.drop_column("description")
