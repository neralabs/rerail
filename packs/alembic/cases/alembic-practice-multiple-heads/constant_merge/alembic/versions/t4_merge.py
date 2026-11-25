"""merge t2 and t3, both named by module constants: one head"""

LEFT = "t2"
RIGHT = "t3"

revision = "t4"
down_revision = (LEFT, RIGHT)


def upgrade():
    pass


def downgrade():
    pass
