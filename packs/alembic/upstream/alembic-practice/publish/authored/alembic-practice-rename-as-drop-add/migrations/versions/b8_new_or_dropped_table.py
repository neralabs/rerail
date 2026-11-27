"""tables created or dropped in the same upgrade"""
import sqlalchemy as sa
from alembic import op

revision = "b8"
down_revision = "b7"
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("label", sa.String(40)))
    op.add_column("tags", sa.Column("name", sa.String(40)))
    op.drop_column("tags", "label")
    op.add_column("legacy_tags", sa.Column("name", sa.String(40)))
    op.drop_column("legacy_tags", "label")
    op.drop_table("legacy_tags")


def downgrade():
    op.drop_table("tags")
