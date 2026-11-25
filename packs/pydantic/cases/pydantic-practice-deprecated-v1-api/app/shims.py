import importlib.metadata

import pydantic as pd
from fastapi._compat import PYDANTIC_V2
from pydantic import VERSION as PV

from app import compat
from app.compat import API_V1_STR, MAJOR
from app.compat import PYDANTIC_V2 as IS_V2
from app.config import settings
from app.schemas import Account, User


def by_version(acct: Account):
    if PV.startswith("1."):
        return acct.dict()
    return acct.model_dump()


def by_module_version(acct: Account):
    if pd.VERSION[0] == "1":
        return acct.dict()
    return acct.model_dump()


def by_imported_flag(acct: Account):
    if not IS_V2:
        return acct.dict()
    return acct.model_dump()


def by_module_flag(acct: Account):
    if compat.IS_V1:
        return acct.dict()
    return acct.model_dump()


def by_major(acct: Account):
    if MAJOR >= 2:
        return acct.model_dump()
    else:
        return acct.dict()


def by_library_flag(acct: Account):
    if not PYDANTIC_V2:
        return acct.dict()
    return acct.model_dump()


def by_distribution(acct: Account):
    if importlib.metadata.version("pydantic").startswith("1"):
        return acct.dict()
    return acct.model_dump()


def by_probe(acct: Account):
    return acct.model_dump() if hasattr(acct, "model_dump") else acct.dict()


def by_getattr(data: dict):
    if getattr(User, "model_validate", None) is None:
        return User.parse_obj(data)
    return User.model_validate(data)


def by_elif(acct: Account, fmt: str):
    if fmt == "short":
        return acct.model_dump(include={"id"})
    elif not IS_V2:
        return acct.dict()
    return acct.model_dump()


def by_local_flag(acct: Account):
    v2 = hasattr(acct, "model_dump")
    if not v2:
        return acct.dict()
    return acct.model_dump()


def versioned(path: str, body: Account) -> dict:
    if path.startswith(settings.API_V1_STR):
        return body.dict()  # caught
    return body.model_dump()


def by_api_path(path: str, acct: Account):
    if path.startswith(API_V1_STR):
        return acct.dict()  # caught
    return acct.model_dump()


def by_setting(acct: Account):
    if not settings.USE_V2_FORMAT:
        return acct.dict()  # caught
    return acct.model_dump()


def by_api_version(acct: Account, version: int):
    if version >= 2:
        return acct.dict()  # caught
    return acct.model_dump()


def by_v1_flag(acct: Account, use_v1_format: bool):
    return acct.dict() if use_v1_format else acct.model_dump()  # caught
