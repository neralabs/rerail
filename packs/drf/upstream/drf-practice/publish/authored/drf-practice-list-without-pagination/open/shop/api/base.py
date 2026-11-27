from rest_framework import generics, viewsets
from rest_framework.pagination import PageNumberPagination


class Pages(PageNumberPagination):
    page_size = 50
    max_page_size = 500


class PagedViewSet(viewsets.ModelViewSet):
    pagination_class = Pages


class ProjectListView(generics.ListAPIView):
    """Base list view: its subclasses are judged, not it."""


class SelfPagingViewSet(viewsets.ReadOnlyModelViewSet):
    def paginate_queryset(self, queryset):
        return list(queryset[: int(self.request.query_params.get("n", 20))])


def paginator(queryset, request):
    page = int(request.query_params.get("page", 1))
    return queryset[(page - 1) * 50 : page * 50]
