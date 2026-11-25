import os

from django.http import HttpResponse


def convert(request, name):
    os.system(f"convert {name} out.png")  # caught
    return HttpResponse()


def thumb(request, size):
    # `<int:size>`: digits only.
    os.system(f"convert in.png -resize {size} out.png")
    return HttpResponse()
