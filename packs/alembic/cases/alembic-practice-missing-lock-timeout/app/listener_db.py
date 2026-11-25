from sqlalchemy import create_engine, event

engine = create_engine("postgresql://app@db/app")


@event.listens_for(engine, "connect")
def _set_timeouts(dbapi_connection, connection_record):
    cursor = dbapi_connection.cursor()
    cursor.execute("SET lock_timeout = '5s'")
    cursor.close()
