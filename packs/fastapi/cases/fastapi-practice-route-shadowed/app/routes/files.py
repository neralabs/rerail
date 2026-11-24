from fastapi import APIRouter

router = APIRouter()


# A `:path` parameter swallows every segment after it.
@router.get("/files/{file_path:path}")
def read_file(file_path: str):
    return {"path": file_path}


@router.get("/files/recent")  # caught
def recent_files():
    return []


@router.get("/files/{file_id}/meta")  # caught
def file_meta(file_id: str):
    return {}


# `:int` matches digits only: a word literal stays reachable, a digits literal does not.
@router.get("/pages/{page:int}")
def page(page: int):
    return {}


@router.get("/pages/latest")
def latest_page():
    return {}


@router.get("/pages/0")  # caught
def first_page():
    return {}


@router.get("/pages/{n:int}")  # caught
def page_again(n: int):
    return {}


# `{slug}` (or `{slug:str}`) matches every segment, digits too.
@router.get("/tags/{slug:str}")
def tag(slug: str):
    return {}


@router.get("/tags/{tag_id:int}")  # caught
def tag_by_id(tag_id: int):
    return {}


# `:float` and `:uuid` do not match a word.
@router.get("/scores/{value:float}")
def score(value: float):
    return {}


@router.get("/scores/top")
def top_scores():
    return []


@router.get("/scores/{value:int}")  # caught
def score_int(value: int):
    return {}


@router.get("/runs/{run_id:uuid}")
def run(run_id: str):
    return {}


@router.get("/runs/active")
def active_runs():
    return []


# A path parameter matches several segments; `{name}` matches one, so it does not shadow it.
@router.get("/blobs/{key}")
def blob(key: str):
    return {}


@router.get("/blobs/{key_path:path}")
def blob_path(key_path: str):
    return {}


# api_route by its methods; GET when it lists none.
@router.api_route("/hooks/{name}", methods=["GET", "POST"])
def hook(name: str):
    return {}


@router.post("/hooks/github")  # caught
def github_hook():
    return {}


@router.put("/hooks/github")
def put_github_hook():
    return {}


@router.api_route("/jobs/{job_id}")
def job(job_id: str):
    return {}


@router.get("/jobs/queue")  # caught
def job_queue():
    return []


@router.api_route("/jobs/stats", methods=["POST"])
def job_stats():
    return {}


# Routes registered by hand.
def list_reports(name: str):
    return []


def report_summary():
    return {}


router.add_api_route("/reports/{name}", list_reports)
router.add_api_route("/reports/summary", report_summary)  # caught


# HEAD and GET are separate methods; a websocket only meets websockets.
@router.head("/status/{probe}")
def head_probe(probe: str):
    return None


@router.get("/status/live")
def live():
    return {}


@router.websocket("/status/{channel}")
async def channel(ws):
    return None


@router.get("/status/ready")
def ready():
    return {}


@router.websocket("/status/events")  # caught
async def events(ws):
    return None
