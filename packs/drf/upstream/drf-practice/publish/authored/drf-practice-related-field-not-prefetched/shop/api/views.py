from django.db.models import Prefetch
from rest_framework import generics, mixins, viewsets
from rest_framework.views import APIView

from core.views import BookBaseView
from shop.api import serializers as s
from shop.api.serializers import (
    ArticleSerializer,
    BookAllSerializer,
    BookAuthorNameSerializer,
    BookChaptersSerializer,
    BookCustomSerializer,
    BookDepthSerializer,
    BookNestedChildSerializer,
    BookNestedSerializer,
    BookPlainSerializer,
    BookPublisherSerializer,
    BookReviewsSerializer,
    BookTagsSerializer,
    BookTitleSerializer,
    BookWriteSerializer,
)
from shop.models import Article, Book, Review


def visible_books(user):
    return Book.objects.filter(active=True)


class BookViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.all()  # caught
    serializer_class = BookNestedSerializer


class BookPublisherList(generics.ListAPIView):
    queryset = Book.objects.filter(active=True).order_by("title")  # caught
    serializer_class = BookPublisherSerializer


class BookTagsList(generics.ListAPIView):
    queryset = Book.objects.select_related("author")  # caught
    serializer_class = BookTagsSerializer


class BookDepthList(generics.ListAPIView):
    queryset = Book.objects.all()  # caught
    serializer_class = BookDepthSerializer


class BookReviewsViewSet(viewsets.ReadOnlyModelViewSet):
    serializer_class = BookReviewsSerializer

    def get_queryset(self):  # caught
        qs = Book.objects.filter(active=True)
        if self.request.query_params.get("q"):
            qs = qs.filter(title__icontains=self.request.query_params["q"])
        return qs


class BookChaptersList(generics.ListAPIView):
    queryset = Book.objects.select_related()  # caught
    serializer_class = BookChaptersSerializer


class BookAuthorNameList(generics.ListCreateAPIView):
    queryset = Book.objects.select_related("publisher")  # caught
    serializer_class = s.BookAuthorNameSerializer


class BookAllList(generics.ListAPIView):
    queryset = Book.objects.select_related("author", "publisher")  # caught
    serializer_class = BookAllSerializer


# The queryset comes from a base view in another module: reported at the serializer choice.
class BookFromBaseList(mixins.ListModelMixin, BookBaseView):
    serializer_class = BookNestedChildSerializer  # caught


# get_queryset reads the base queryset through super(): reported at get_queryset.
class BookSuperList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookNestedSerializer

    def get_queryset(self):  # caught
        return super().get_queryset().filter(active=True)


# ---- near misses ----------------------------------------------------------------------------------------------------


class BookLoadedViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.select_related("author")
    serializer_class = BookNestedSerializer


class BookDeepLoadList(generics.ListAPIView):
    queryset = Book.objects.select_related("author__country")
    serializer_class = BookNestedSerializer


class BookTagsLoadedList(generics.ListAPIView):
    queryset = Book.objects.prefetch_related(Prefetch("tags"))
    serializer_class = BookTagsSerializer


class BookReviewsLoadedList(generics.ListAPIView):
    serializer_class = BookReviewsSerializer

    def get_queryset(self):
        qs = Book.objects.all()
        return qs.prefetch_related("reviews")


class BookAllLoadedList(generics.ListAPIView):
    queryset = Book.objects.select_related().prefetch_related("tags")
    serializer_class = BookAllSerializer


class BookPlainList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookPlainSerializer


class BookWriteList(generics.ListCreateAPIView):
    queryset = Book.objects.all()
    serializer_class = BookWriteSerializer


class BookCustomList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookCustomSerializer


class BookTitleList(generics.ListAPIView):
    queryset = Book.objects.all()
    serializer_class = BookTitleSerializer


class ArticleList(generics.ListAPIView):
    queryset = Article.objects.all()
    serializer_class = ArticleSerializer


# Detail only: one row, no loop.
class BookDetail(generics.RetrieveUpdateAPIView):
    queryset = Book.objects.all()
    serializer_class = BookNestedSerializer


class BookAPIView(APIView):
    serializer_class = BookNestedSerializer


# A queryset the rule cannot see: a helper, a custom queryset method, a related manager.
class BookHelperList(generics.ListAPIView):
    serializer_class = BookNestedSerializer

    def get_queryset(self):
        return visible_books(self.request.user)


class BookCustomMethodList(generics.ListAPIView):
    queryset = Book.objects.with_stats()
    serializer_class = BookNestedSerializer


class UserBookList(generics.ListAPIView):
    serializer_class = BookNestedSerializer

    def get_queryset(self):
        return self.request.user.books.all()


# The list action may use another serializer.
class BookSwitchViewSet(viewsets.ModelViewSet):
    queryset = Book.objects.all()
    serializer_class = BookNestedSerializer

    def get_serializer_class(self):
        if self.action == "list":
            return BookPlainSerializer
        return BookNestedSerializer


# A subclass of a reported list view, with nothing of its own: the base is reported, once.
class BookViewSetV2(BookViewSet):
    pass


# The serializer's model is not the queryset's.
class ReviewBooksList(generics.ListAPIView):
    queryset = Review.objects.all()
    serializer_class = BookNestedSerializer
