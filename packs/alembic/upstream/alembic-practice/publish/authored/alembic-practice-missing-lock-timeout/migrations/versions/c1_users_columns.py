"""users: columns and a foreign key, no lock_timeout"""
import sqlalchemy as sa
from alembic import op

revision = "c1"
down_revision = None
branch_labels = None
depends_on = None


def upgrade():
    op.add_column("users", sa.Column("nickname", sa.String(50), nullable=True))  # caught
    op.alter_column("users", "email", existing_type=sa.String(100), type_=sa.String(255))
    op.create_foreign_key("fk_users_team", "users", "teams", ["team_id"], ["id"])


def downgrade():
    op.drop_constraint("fk_users_team", "users", type_="foreignkey")
    op.drop_column("users", "nickname")
