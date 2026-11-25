from django.urls import path, re_path

from docs import media_views

urlpatterns = [
    path("files/<path:relpath>", media_views.serve),
    path("pages/<str:name>", media_views.page),
    path("pages/<slug:name>/raw", media_views.page_raw),
    path("themes/<name>", media_views.theme),
    path("exports/<int:export_id>", media_views.export),
    re_path(r"^legacy/(?P<relpath>.+)$", media_views.legacy),
    path("dl/<str:name>", media_views.DownloadView.as_view()),
    path("search", media_views.download_q),
]
