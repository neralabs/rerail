"""set: a triple-quoted statement"""
import sqlalchemy as sa
from alembic import op

revision = "s7"
down_revision = "s6"
branch_labels = None
depends_on = None


def upgrade():
    op.execute(
        """
        SET LOCAL lock_timeout = '4s'
        """
    )
    op.drop_constraint("fk_jobs_owner", "jobs", type_="foreignkey")


def downgrade():
    pass
