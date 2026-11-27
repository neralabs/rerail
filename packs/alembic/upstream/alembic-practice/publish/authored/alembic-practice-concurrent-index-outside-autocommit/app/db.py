import sqlalchemy as sa

engine = sa.create_engine("postgresql+psycopg://localhost/app")
metadata = sa.MetaData()

CONCURRENT_INDEXES = True
