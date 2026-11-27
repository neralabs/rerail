"""on v1, which a squash removed: a parent outside the run"""
import sqlalchemy as sa
from alembic import op

revision = "v2"
down_revision = "v1"


def upgrade():
    op.add_column("users", sa.Column("email", sa.String(255)))


def downgrade():
    op.drop_column("users", "email")
