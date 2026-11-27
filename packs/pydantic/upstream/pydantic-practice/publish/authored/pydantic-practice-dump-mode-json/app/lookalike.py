# The project's own classes that share names with Starlette's and aiohttp's, and encode with orjson.
import orjson


class JSONResponse:
    def __init__(self, content) -> None:
        self.body = orjson.dumps(content)


class ClientSession:
    def post(self, url: str, json=None) -> bytes:
        return orjson.dumps(json)
