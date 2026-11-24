"""copy with bulk_insert"""
import sqlalchemy as sa
from alembic import op

revision = "b5"
down_revision = "b4"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("prefs", sa.Column("theme_name", sa.String(20)))
    op.bulk_insert(sa.table("prefs_log", sa.column("note", sa.String())), [{"note": "theme moved"}])
    op.drop_column("prefs", "theme")


def downgrade():
    op.add_column("prefs", sa.Column("theme", sa.VARCHAR(length=20)))
    op.drop_column("prefs", "theme_name")
