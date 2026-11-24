from rest_framework.pagination import PageNumberPagination


class NotePages(PageNumberPagination):
    page_size = 50
