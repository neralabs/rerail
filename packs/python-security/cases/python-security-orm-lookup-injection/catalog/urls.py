from django.urls import path

from catalog import routed

urlpatterns = [
    path("books/by/<str:sort>/", routed.sorted_by),
    path("books/at/<int:position>/", routed.by_position),
    path("books/view/<slug:sort>/", routed.SortedView.as_view()),
]
