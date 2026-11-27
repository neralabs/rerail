"""add hero.name back"""
import sqlalchemy as sa
from alembic import op

revision: str = "a6"
down_revision: str | None = "a5"
branch_labels = None


def upgrade():
    op.add_column("hero", sa.Column("name", sa.String(255), nullable=True))


def downgrade():
    op.drop_column("hero", "name")
