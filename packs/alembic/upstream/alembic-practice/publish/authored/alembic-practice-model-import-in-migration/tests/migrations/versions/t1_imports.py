"""a migration in a test tree"""
from alembic import op

from app.models.user import User

revision = "t1"
down_revision = None
branch_labels = None


def upgrade():
    pass


def downgrade():
    pass
