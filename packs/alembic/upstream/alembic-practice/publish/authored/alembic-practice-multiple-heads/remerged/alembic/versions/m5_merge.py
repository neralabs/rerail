"""the same merge, written on another branch"""

revision = "m5"
down_revision = ("m3", "m2")  # caught
branch_labels = None
depends_on = None


def upgrade():
    pass


def downgrade():
    pass
