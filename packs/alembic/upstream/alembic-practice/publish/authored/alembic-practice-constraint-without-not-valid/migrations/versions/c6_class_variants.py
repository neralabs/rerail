"""constraint classes and flags through aliases, constants and look-alike libraries"""
import sqlalchemy
from alembic import op as migrate
from sqlalchemy import CheckConstraint as Check, Column, ForeignKey as FK, Integer

from app.schema import ForeignKey as AppForeignKey

revision = "c6"
down_revision = "c5"
branch_labels = None

VALIDATE_LATER = True
VALIDATE_NOW = False


def upgrade():
    migrate.add_column("orders", Column("agent_id", Integer(), FK("agents.id")))  # caught
    migrate.add_column("orders", sqlalchemy.Column("qty", Integer(), Check("qty > 0")))  # caught
    migrate.create_foreign_key("fk_orders_zone", "orders", "zones", ["zone_id"], ["id"], postgresql_not_valid=VALIDATE_NOW)  # caught
    migrate.create_foreign_key("fk_orders_area", "orders", "areas", ["area_id"], ["id"], postgresql_not_valid=VALIDATE_LATER)
    # A class of the project's own that shares the name: no SQLAlchemy constraint.
    migrate.add_column("orders", Column("ref_id", Integer(), AppForeignKey("refs.id")))


def downgrade():
    pass
