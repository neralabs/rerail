from sqlalchemy import Column, Integer, MetaData, String, Table

metadata = MetaData()

sessions_table = Table(
    "sessions",
    metadata,
    Column("id", Integer, primary_key=True),
    Column("token", String),
    Column("ip", String),
)
