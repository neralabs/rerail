from django.db import transaction
from django.db.models import F
from django.shortcuts import get_object_or_404

from .models import Article, Wallet


def view(request, pk):
    article = Article.objects.get(pk=pk)
    article.hits += 1  # caught
    article.save()


def view_404(request, pk):
    article = get_object_or_404(Article, pk=pk)
    article.likes = article.likes - 1  # caught
    article.save()


def loop(request):
    for w in Wallet.objects.filter(amount__gt=0):
        w.amount -= 10  # caught
        w.save()


def first(request):
    w = Wallet.objects.filter(pk=1).first()
    w.amount = 5 + w.amount  # caught
    w.save()


def counted(request, day):
    w, created = Wallet.objects.get_or_create(pk=day)
    if not created:
        w.amount += 1  # caught
        w.save()


def locked(request, pk):
    with transaction.atomic():
        w = Wallet.objects.select_for_update().get(pk=pk)
        w.amount -= 10
        w.save()


def with_f(request, pk):
    Wallet.objects.filter(pk=pk).update(amount=F("amount") + 1)
    w = Wallet.objects.get(pk=pk)
    w.amount = F("amount") + 1
    w.save()


def new_row(request):
    w = Wallet(amount=1)
    w.amount += 1
    w.save()


def unknown(request, obj):
    obj.amount += 1
    obj.save()


def not_numeric(request, pk):
    a = Article.objects.get(pk=pk)
    a.title = a.title + "!"
    a.save()


def no_save(request, pk):
    a = Article.objects.get(pk=pk)
    a.likes += 1
    return a.likes


def other_save(request, pk):
    a = Article.objects.get(pk=pk)
    b = Article.objects.get(pk=pk + 1)
    a.likes += 1
    b.save()


def set_not_from_self(request, pk):
    a = Article.objects.get(pk=pk)
    a.likes = request.GET["n"]
    a.save()
