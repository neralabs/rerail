from django.urls import path

from netops import kwviews

urlpatterns = [
    path("convert/<str:name>/", kwviews.convert),
    path("thumb/<int:size>/", kwviews.thumb),
]
