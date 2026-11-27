from fastapi import APIRouter, Depends

from acme.auth import get_current_user
from acme.billing.views import router as billing_router

api_router = APIRouter()
authenticated_api_router = APIRouter()
authenticated_api_router.include_router(billing_router, prefix="/billing")
api_router.include_router(authenticated_api_router, dependencies=[Depends(get_current_user)])
