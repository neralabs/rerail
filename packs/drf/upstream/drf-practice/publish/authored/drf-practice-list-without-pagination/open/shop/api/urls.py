from django.urls import path
from rest_framework.routers import DefaultRouter

from shop.api import views

router = DefaultRouter()
router.register("routed-orders", views.RoutedOrders)

urlpatterns = [
    path("profile/<int:pk>/", views.ProfileViewSet.as_view({"get": "retrieve", "patch": "partial_update"})),
    path("board/", views.InvoiceBoardViewSet.as_view({"get": "list", "post": "create"})),
    path("routed-orders/<int:pk>/", views.RoutedOrders.as_view({"get": "retrieve"})),
    *router.urls,
]
