"""a column replaced by a shorter one: a different type, not a rename"""
import sqlalchemy as sa
from alembic import op

revision = "b12"
down_revision = "b11"
branch_labels = None


def upgrade():
    op.add_column("vouchers", sa.Column("short_code", sa.String(length=8), nullable=True))
    op.drop_column("vouchers", "code")


def downgrade():
    op.add_column("vouchers", sa.Column("code", sa.VARCHAR(length=80), nullable=True))
    op.drop_column("vouchers", "short_code")
