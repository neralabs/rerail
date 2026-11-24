from starlette.middleware.cors import CORSMiddleware


class SandboxCORSMiddleware(CORSMiddleware):
    async def __call__(self, scope, receive, send):
        await super().__call__(scope, receive, send)
