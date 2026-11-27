from sqlalchemy.ext.asyncio import create_async_engine

async_engine = create_async_engine(
    "postgresql+asyncpg://app@db/app",
    connect_args={"server_settings": {"lock_timeout": "5s", "application_name": "app"}},
)
