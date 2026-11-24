import re

from starlette.middleware.cors import CORSMiddleware


class BrowserCORSMiddleware(CORSMiddleware):
    async def __call__(self, scope, receive, send):
        # Sandboxed pages answer their own CORS.
        if re.match(r"^/sandbox/", scope.get("path", "")):
            await self.app(scope, receive, send)
        else:
            await super().__call__(scope, receive, send)


class CORSConfig:
    """The project's own CORS options: not a middleware."""

    def __init__(self, allow_origins=(), allow_credentials=False):
        self.allow_origins = allow_origins
        self.allow_credentials = allow_credentials
