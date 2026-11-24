from fastapi import APIRouter

from app.deps import ProjectsWrite

router = APIRouter(prefix="/projects")


@router.post("/")
async def create_project(auth: ProjectsWrite):
    return {}


@router.patch("/{project_id}")
async def update_project(project_id: int, auth: ProjectsWrite):
    return {}


@router.delete("/{project_id}")
async def delete_project(project_id: int, auth: ProjectsWrite):
    return {}


@router.post("/{project_id}/archive")
async def archive_project(project_id: int):  # caught
    return {}
