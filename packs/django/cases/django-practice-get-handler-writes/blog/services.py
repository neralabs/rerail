from .models import Post, Visit


def record_visit(post):
    Visit.objects.create(post=post)


def audit(post):
    record_visit(post)


def load(pk):
    return Post.objects.get(pk=pk)


def bump_post(pk):
    post = Post.objects.get(pk=pk)
    post.bump()


def lookup(pk):
    return Post.objects.filter(pk=pk).first()
