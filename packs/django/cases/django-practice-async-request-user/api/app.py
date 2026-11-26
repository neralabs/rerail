from starlette.requests import Request
from starlette.responses import JSONResponse


# Starlette's request.user is a plain attribute set by its AuthenticationMiddleware: no lazy Django object.
async def me(request: Request):
    return JSONResponse({"name": request.user.display_name})
