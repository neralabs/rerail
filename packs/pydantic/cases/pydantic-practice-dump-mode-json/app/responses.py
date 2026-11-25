import orjson
from fastapi.encoders import jsonable_encoder
from fastapi.responses import JSONResponse
from starlette.responses import JSONResponse as StarletteJSONResponse


# Keeps Starlette's render: still json.dumps.
class AppJSONResponse(JSONResponse):
    media_type = "application/vnd.app+json"


class VersionedResponse(AppJSONResponse):
    media_type = "application/vnd.app.v2+json"


# Renders with orjson, which encodes datetime, UUID and dataclasses.
class FastResponse(StarletteJSONResponse):
    def render(self, content) -> bytes:
        return orjson.dumps(content)


class FastChildResponse(FastResponse):
    media_type = "application/x-fast+json"


# Converts the content before Starlette renders it.
class EncodedResponse(JSONResponse):
    def __init__(self, content, **kwargs) -> None:
        super().__init__(jsonable_encoder(content), **kwargs)
