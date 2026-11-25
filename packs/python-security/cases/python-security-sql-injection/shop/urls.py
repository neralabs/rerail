from django.urls import path, re_path

from shop import kwviews
from shop.kwviews import ProductDetail, ReportView

urlpatterns = [
    path("products/<slug:slug>/", kwviews.by_slug),
    path("products/<int:pk>/", kwviews.by_pk),
    re_path(r"^archive/(?P<year>[0-9]{4})/(?P<label>[^/]+)/$", kwviews.by_year),
    path("reports/<str:name>/", ReportView.as_view()),
    path("detail/<int:pk>/", ProductDetail.as_view()),
]
