import os
from dataclasses import dataclass


# Not Pydantic: a plain config class and a dataclass.
class Config:
    SECRET_KEY: str = os.environ.get("SECRET_KEY", "")


@dataclass
class MailConfig:
    password: str = ""
