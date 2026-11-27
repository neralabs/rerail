import re
from typing import Annotated, Any

from pydantic import AfterValidator, BeforeValidator


# Attached through `Annotated` in app/annotated.py: the first parameter is the raw input.
def normalize_claim_code(value: str) -> str:
    return "-".join(part for part in re.split(r"[\s\-_]+", value.strip().lower()) if part)  # caught


def upper_if_str(value: Any) -> Any:
    if isinstance(value, str):
        return value.upper()
    return value


def lower_slug(value: str) -> str:
    return value.lower()  # caught


# An after validator gets the field's validated value: a str here.
def no_spaces(value: str) -> str:
    if " " in value:
        raise ValueError("no spaces")
    return value


# A module-level alias another module's fields use.
Slug = Annotated[str, BeforeValidator(lower_slug), AfterValidator(no_spaces)]


# Used only by a model the project builds itself (app/annotated.py's Internal): left alone.
def trim_internal(value: str) -> str:
    return value.strip()
