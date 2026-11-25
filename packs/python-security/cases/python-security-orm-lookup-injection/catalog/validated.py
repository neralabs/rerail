from django import forms
from django.http import JsonResponse
from rest_framework import serializers
from rest_framework.response import Response
from rest_framework.views import APIView

from catalog.models import Book

SORTS = [("title", "Title"), ("-year", "Newest")]


class ListParams(serializers.Serializer):
    ordering = serializers.CharField(required=False)
    fields = serializers.ListField(child=serializers.CharField())
    sort = serializers.ChoiceField(choices=SORTS)


class SortForm(forms.Form):
    sort = forms.CharField()
    choice = forms.ChoiceField(choices=SORTS)


class BookParams(APIView):
    def get(self, request):
        p = ListParams(data=request.query_params)
        p.is_valid(raise_exception=True)
        books = Book.objects.all().order_by(p.validated_data["ordering"])  # caught
        books = Book.objects.values(*p.validated_data["fields"])  # caught
        books = Book.objects.order_by(p.validated_data.get("ordering"))  # caught
        # A choice field's value is one of the fixed choices; the validated keys are the declared fields.
        books = Book.objects.order_by(p.validated_data["sort"])
        books = Book.objects.filter(**p.validated_data)
        return Response({"n": books.count()})


def form_sort(request):
    form = SortForm(request.GET)
    if not form.is_valid():
        return JsonResponse({}, status=400)
    books = Book.objects.order_by(form.cleaned_data["sort"])  # caught
    books = books.order_by(form.cleaned_data["choice"])
    books = books.filter(**form.cleaned_data)
    return JsonResponse({"n": books.count()})
