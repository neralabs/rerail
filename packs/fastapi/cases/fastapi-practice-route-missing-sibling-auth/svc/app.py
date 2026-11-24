from fastapi import APIRouter, Depends, FastAPI, Header
from fastapi.security import HTTPBearer

bearer = HTTPBearer()


def verify_token(authorization: str = Header(...)):
    return authorization


def current_user(creds=Depends(bearer)):
    return creds


# The app requires a token for every route it serves, its routers' included.
app = FastAPI(dependencies=[Depends(verify_token)])


@app.get("/a")
def a(user=Depends(current_user)):
    return {}


@app.get("/b")
def b(user=Depends(current_user)):
    return {}


@app.post("/c")
def c(user=Depends(current_user)):
    return {}


@app.delete("/d")
def d():
    return {}


tools = APIRouter()


@tools.get("/t1")
def t1(user=Depends(current_user)):
    return {}


@tools.get("/t2")
def t2(user=Depends(current_user)):
    return {}


@tools.post("/t3")
def t3(user=Depends(current_user)):
    return {}


@tools.post("/t4")
def t4():
    return {}


app.include_router(tools, prefix="/tools")

# A second app with no global dependency: its HTTPBearer siblings make the bare route stand out.
api = FastAPI()


@api.get("/v1/jobs")
def list_jobs(creds=Depends(bearer)):
    return []


@api.post("/v1/jobs")
def create_job(creds=Depends(bearer)):
    return {}


@api.delete("/v1/jobs/{i}")
def delete_job(i: int, creds=Depends(bearer)):
    return {}


@api.post("/v1/jobs/{i}/retry")
def retry_job(i: int):  # caught
    return {}
