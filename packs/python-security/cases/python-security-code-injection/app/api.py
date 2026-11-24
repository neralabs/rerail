import pickle

import yaml
from fastapi import APIRouter, Depends, UploadFile
from pydantic import BaseModel

from app import services, titles

router = APIRouter()


class PromptTest(BaseModel):
    template: str
    variables: dict


@router.post("/prompts/test")
async def test_prompt(body: PromptTest):
    return {"text": services.render_prompt(body.template, body.variables)}


@router.post("/prompts/render")
async def render_saved(body: PromptTest):
    return {"sandboxed": services.render_sandboxed(body.template, {}), "title": titles.render_title(body.template),
            "path": titles.render_path(body.template), "text": services.render_saved(body.variables)}


@router.post("/import")
async def import_state(file: UploadFile):
    raw = await file.read()
    state = pickle.loads(raw)  # caught
    config = yaml.load(raw, Loader=yaml.SafeLoader)
    return {"state": state, "config": config}


@router.get("/expr")
async def evaluate(expr: str, settings=Depends(services.get_settings)):
    return {"value": eval(expr), "setting": eval(settings.default_expr)}  # caught
