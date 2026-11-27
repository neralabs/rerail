from rest_framework import generics, serializers, viewsets
from rest_framework.filters import BaseFilterBackend
from rest_framework.response import Response
from rest_framework.views import APIView

from catalog.models import Book

ORDERING_FIELDS = {"title", "-title", "year", "-year"}


class BookSerializer(serializers.ModelSerializer):
    class Meta:
        model = Book
        fields = ["title", "year"]


class BookViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.all()
    serializer_class = BookSerializer

    def get_queryset(self):
        qs = super().get_queryset()
        return qs.filter(**self.request.query_params.dict())  # caught


class BookList(generics.ListAPIView):
    serializer_class = BookSerializer

    def post(self, request):
        books = Book.objects.filter(**request.data)  # caught
        ordering = request.query_params.get("ordering")
        books = self.get_queryset().order_by(ordering)  # caught
        Book.objects.filter(pk__in=request.data["ids"]).update(**request.data["changes"])  # caught
        return Response({"n": books.count()})


class BookOrderingFilter(BaseFilterBackend):
    def filter_queryset(self, request, queryset, view):
        ordering = request.query_params.get("ordering", "title")
        return queryset.order_by(ordering)  # caught


class SafeOrderingFilter(BaseFilterBackend):
    def filter_queryset(self, request, queryset, view):
        ordering = request.query_params.get("ordering", "title")
        if ordering in ORDERING_FIELDS:
            return queryset.order_by(ordering)
        return queryset


class BookUpsert(APIView):
    def post(self, request):
        serializer = BookSerializer(data=request.data)
        serializer.is_valid(raise_exception=True)
        # A serializer's validated data has its declared field names.
        book, _ = Book.objects.get_or_create(**serializer.validated_data)
        Book.objects.filter(pk=book.pk).update(**serializer.validated_data)
        return Response({"id": book.pk})
