from pydantic import BaseModel, Field, field_validator

LOCALES = {"en-US", "de-DE"}


class Registration(BaseModel):
    username: str
    locale: str = "en-US"
    theme: str = Field("light", pattern=r"^[a-z]+$")
    avatar: str = ""

    @field_validator("locale")
    def valid_locale(cls, v):
        if v not in LOCALES:
            raise ValueError("invalid locale")
        return v
