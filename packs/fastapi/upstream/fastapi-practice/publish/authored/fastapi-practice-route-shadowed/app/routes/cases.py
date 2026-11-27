from fastapi import APIRouter

router = APIRouter(prefix="/cases")


@router.get("/{case_id}")
def get_case(case_id: int):
    return {"id": case_id}


@router.get("/minimal")  # caught
def get_cases_minimal():
    return []


@router.get("/{case_id}/participants/{participant_id}")
def get_participant(case_id: int, participant_id: int):
    return {}


# The same pattern under another parameter name: a duplicate.
@router.get(  # caught
    "/{case_id}/participants/{pid}",
    summary="Participant",
)
def get_participant_again(case_id: int, pid: int):
    return {}


@router.put("/{case_id}")
def update_case(case_id: int):
    return {}


@router.put("/bulk")  # caught
def update_cases_bulk():
    return {}


# A different method: GET /minimal is shadowed, DELETE /minimal is not.
@router.delete("/minimal")
def delete_minimal():
    return {}


# A different number of segments.
@router.get("/{case_id}/events")
def get_case_events(case_id: int):
    return []


@router.get("/minimal/events")  # caught
def get_minimal_events():
    return []


@router.post("/")
def create_case():
    return {}


@router.post("/")  # caught
async def create_case_v2():
    return {}
