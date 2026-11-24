from pydantic_settings import BaseSettings

revision = "0002"
down_revision = "0001"


# A migration reads the legacy environment once to move it into the database: not the process's settings object.
class LegacyOpenAISettings(BaseSettings):
    OPENAI_API_KEY: str | None = None


def upgrade() -> None:
    LegacyOpenAISettings()
