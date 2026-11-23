from django.db.models import F
from django.shortcuts import get_object_or_404, render
from django.views import View
from django.views.decorators.http import require_GET, require_http_methods, require_POST, require_safe
from django.views.generic import DetailView, ListView
from rest_framework import viewsets
from rest_framework.decorators import api_view

from .models import Post
from .services import audit, bump_post, load, lookup, record_visit


class PostDetail(DetailView):
    model = Post

    def get(self, request, pk):  # caught
        Post.objects.filter(pk=pk).update(views=F("views") + 1)
        return super().get(request, pk=pk)


class PostAudit(View):
    def get(self, request, pk):  # caught
        audit(load(pk))
        return render(request, "post.html")

    def post(self, request, pk):
        Post.objects.create(title=request.POST["title"])
        return render(request, "post.html")


class PostBump(View):
    def head(self, request, pk):  # caught
        bump_post(pk)
        return render(request, "post.html")


class PostList(ListView):
    model = Post

    def get(self, request):
        return render(request, "list.html", {"posts": Post.objects.filter(views__gt=0)})


class PostViewSet(viewsets.ModelViewSet):
    def retrieve(self, request, pk=None):  # caught
        post = Post.objects.get(pk=pk)
        post.views += 1
        post.save()
        return render(request, "post.html")

    def list(self, request):
        return render(request, "list.html", {"post": lookup(1)})

    def create(self, request):
        Post.objects.create(title="x")


class NotAView:
    def get(self, pk):
        Post.objects.create(title="x")


@require_GET
def detail(request, pk):  # caught
    record_visit(load(pk))
    return render(request, "post.html")


@require_safe
def preview(request, pk):  # caught
    p = Post(title="draft")
    p.save()
    return render(request, "post.html")


@require_http_methods(["GET", "HEAD"])
def touch(request, pk):  # caught
    Post.objects.get_or_create(title="home")
    return render(request, "post.html")


@require_http_methods(["GET", "POST"])
def edit(request, pk):
    Post.objects.create(title="x")
    return render(request, "post.html")


@require_POST
def publish(request, pk):
    Post.objects.create(title="x")
    return render(request, "post.html")


@api_view(["GET"])
def api_detail(request, pk):  # caught
    Post.objects.filter(pk=pk).delete()


@api_view()
def api_default(request, pk):  # caught
    record_visit(load(pk))


@api_view(["GET", "POST"])
def api_both(request, pk):
    Post.objects.create(title="x")


@require_GET
def read_only(request, pk):
    return render(request, "post.html", {"post": load(pk)})


def undecorated(request):
    Post.objects.create(title="x")


@require_GET
def remove(request, pk):  # caught
    post = get_object_or_404(Post, pk=pk)
    post.delete()
    return render(request, "post.html")


@require_GET
def reset_all(request):  # caught
    for post in Post.objects.all():
        post.views = 0
        post.save()
    return render(request, "post.html")


class PostCount(DetailView):
    model = Post

    def get(self, request, pk):  # caught
        obj = self.get_object()
        obj.views = 1
        obj.save()
        return render(request, "post.html")


class PostQuerysetCount(DetailView):
    queryset = Post.objects.filter(views__gt=0)

    def get(self, request, pk):  # caught
        obj = self.get_object()
        obj.delete()
        return render(request, "post.html")


class Export(View):
    """Django's View never routes a request to `list`: only DRF's viewsets do."""

    def post(self, request):
        self.list()
        return render(request, "post.html")

    def list(self):
        Post.objects.create(title="export")
