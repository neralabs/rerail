import asyncio
import os
import pty
import shlex

from fastapi import APIRouter, Depends
from pydantic import BaseModel

router = APIRouter()


class EnvRequest(BaseModel):
    env_name: str
    python_version: str


def get_workdir() -> str:
    return "/srv/work"


@router.post("/envs")
async def create_env(body: EnvRequest):
    # lollms: a conda environment name from the request body, under a shell.
    proc = await asyncio.create_subprocess_shell(  # caught
        f"conda create -y -n {body.env_name} python={body.python_version}"
    )
    await proc.wait()


@router.get("/version")
async def version(program: str):
    proc = await asyncio.create_subprocess_exec(program, "--version")  # caught
    return await proc.communicate()


@router.post("/exec")
def run_tool(tool: str):
    os.execvp(tool, [tool, "--help"])  # caught


@router.post("/term")
def term(shell_cmd: str):
    pty.spawn(shell_cmd)  # caught


@router.post("/split")
async def split(command: str):
    proc = await asyncio.create_subprocess_exec(*shlex.split(command))  # caught
    return await proc.communicate()


@router.post("/build")
async def build(target: str):
    proc = await asyncio.create_subprocess_exec("bash", "-c", f"make {target}")  # caught
    return await proc.communicate()


# ---- near misses: silent ----------------------------------------------------------------------------------------


@router.get("/du")
async def disk_usage(path: str, workdir: str = Depends(get_workdir)):
    proc = await asyncio.create_subprocess_shell(f"du -sh {workdir}")
    await asyncio.create_subprocess_exec("du", "-sh", path)
    await asyncio.create_subprocess_shell(f"du -sh {shlex.quote(path)}")
    return await proc.communicate()


@router.get("/count/{n}")
def count(n: int):
    os.system(f"seq {n}")
