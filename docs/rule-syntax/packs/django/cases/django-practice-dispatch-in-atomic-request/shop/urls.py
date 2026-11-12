from django.db import transaction
from django.urls import path

from shop import views

urlpatterns = [
    path("orders/", views.place_order),
    path("upload/", transaction.non_atomic_requests(views.upload)),
]
