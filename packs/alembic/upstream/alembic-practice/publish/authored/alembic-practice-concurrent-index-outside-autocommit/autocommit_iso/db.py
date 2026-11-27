from sqlalchemy import create_engine

# Every statement commits on its own: no transaction around the revisions.
engine = create_engine("postgresql+psycopg://localhost/app").execution_options(isolation_level="AUTOCOMMIT")
