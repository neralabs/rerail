from django.shortcuts import render
from django.views import View
from rest_framework.viewsets import GenericViewSet

from .models import Post, Visit


class BaseDetail(View):
    def get(self, request, pk):
        return render(request, "post.html", {"post": self.load(pk)})

    def load(self, pk):
        return Post.objects.get(pk=pk)


class CountingDetail(BaseDetail):  # caught
    def load(self, pk):
        post = Post.objects.get(pk=pk)
        Visit.objects.create(post=post)
        return post


class PlainDetail(BaseDetail):
    def load(self, pk):
        return Post.objects.filter(pk=pk).first()


class Touching(View):
    def get(self, request, pk):  # caught
        Post.objects.filter(pk=pk).update(views=0)
        return render(request, "post.html")


class TouchingChild(Touching):
    template_name = "child.html"


class ListMixin:
    def list(self, request):
        return render(request, "list.html", {"posts": self.get_queryset()})

    def get_queryset(self):
        return Post.objects.all()


class LazyInitViewSet(ListMixin, GenericViewSet):  # caught
    def get_queryset(self):
        Post.objects.get_or_create(title="welcome")
        return Post.objects.all()


class ReadOnlyViewSet(ListMixin, GenericViewSet):
    pass
