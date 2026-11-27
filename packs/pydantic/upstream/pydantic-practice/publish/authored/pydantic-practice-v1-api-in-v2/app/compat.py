from pydantic import VERSION
from pydantic.version import VERSION as PYDANTIC_VERSION

PYDANTIC_V2 = VERSION.startswith("2.")
IS_V1 = not PYDANTIC_V2
MAJOR = int(PYDANTIC_VERSION.split(".")[0])
API_V1_STR = "/api/v1"
