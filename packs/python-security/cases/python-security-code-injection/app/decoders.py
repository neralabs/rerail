import base64
import binascii
import codecs
import gzip
import io
import lzma
import pickle
import zlib

import numpy
import torch
from django.http import HttpResponse


def from_hex_cookie(request):
    return HttpResponse(pickle.loads(bytes.fromhex(request.COOKIES["state"])))  # caught


def from_compressed_body(request):
    return HttpResponse(pickle.loads(zlib.decompress(request.body)))  # caught


def from_gzip_b64(request):
    blob = gzip.decompress(base64.b64decode(request.POST["blob"]))
    return HttpResponse(pickle.loads(blob))  # caught


def from_unhexlify(request):
    return HttpResponse(pickle.loads(binascii.unhexlify(request.GET["s"])))  # caught


def from_codecs(request):
    raw = codecs.decode(request.GET["s"].encode(), "hex")
    return HttpResponse(pickle.loads(lzma.decompress(raw)))  # caught


def from_stream(request):
    return HttpResponse(pickle.load(io.BytesIO(request.body)))  # caught


def array_upload(request):
    arr = numpy.load(request.FILES["arr"], allow_pickle=True)  # caught
    arr = numpy.load(request.FILES["arr"])
    return HttpResponse(arr.shape)


def model_upload(request):
    model = torch.load(request.FILES["model"], weights_only=False)  # caught
    model = torch.load(request.FILES["model"], weights_only=True)
    return HttpResponse(type(model).__name__)


def from_length(request):
    # A length or a checksum of the body carries none of its bytes.
    size = len(zlib.decompress(request.body))
    crc = zlib.crc32(request.body)
    return HttpResponse(pickle.dumps((size, crc)))
