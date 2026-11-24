"""tags: a new table, and data statements"""
import sqlalchemy as sa
from alembic import op

revision = "c5"
down_revision = "c4"
branch_labels = None
depends_on = None


def upgrade():
    op.create_table("tags", sa.Column("id", sa.Integer(), primary_key=True), sa.Column("name", sa.String(50)))
    op.create_index("ix_tags_name", "tags", ["name"], unique=True)
    op.add_column("tags", sa.Column("color", sa.String(7)))
    op.create_foreign_key("fk_tags_owner", "tags", "users", ["owner_id"], ["id"])
    op.execute("UPDATE users SET nickname = name WHERE nickname IS NULL")
    op.bulk_insert(sa.table("tags", sa.column("name")), [{"name": "default"}])


def downgrade():
    op.drop_table("tags")
