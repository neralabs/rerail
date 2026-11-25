from django.urls import path
from rest_framework.routers import DefaultRouter

from board.views import CardViewSet as Cards
from board.views import MemberAdminViewSet, MemberEndpoint

router = DefaultRouter()
router.register("cards", Cards)

urlpatterns = [
    path("me/", MemberEndpoint.as_view({"get": "retrieve", "patch": "partial_update"})),
    path("members/", MemberAdminViewSet.as_view({"get": "list"})),
    *router.urls,
]
