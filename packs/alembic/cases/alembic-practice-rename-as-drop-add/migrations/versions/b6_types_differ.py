"""replace auth_state with an encrypted column of another type"""
import sqlalchemy as sa
from alembic import op

revision = "b6"
down_revision = "b5"
branch_labels = None
depends_on = None


def upgrade():
    op.drop_column("users", "auth_state")
    op.add_column("users", sa.Column("encrypted_auth_state", sa.LargeBinary()))


def downgrade():
    op.drop_column("users", "encrypted_auth_state")
    op.add_column("users", sa.Column("auth_state", sa.JSON()))
