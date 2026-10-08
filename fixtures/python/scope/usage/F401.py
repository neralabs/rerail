from __future__ import annotations

import os
import sys
import json as json
import collections as coll
import pyarrow as pa
import pyarrow.csv
import pyarrow.parquet
import xml.etree
import xml.dom
from typing import List, Dict as Dict
from typing import Optional, Sequence, TYPE_CHECKING
from abc import ABC
from shutil import copy, move
from . import sibling
from .pkg import helper
import re
import re
import string

__all__ = ["copy", "sibling"]
__all__ += ("ABC",)

pa.csv.read_csv
xml.etree
print(helper, re)


def reads_later():
    return move


def local_imports():
    import glob
    import time
    import time
    import fnmatch
    from inspect import signature

    def nested():
        return fnmatch

    del time
    return nested


def shadowed_by_assignment():
    import zipfile
    zipfile = 1
    import tarfile
    print(tarfile)
    tarfile = 2
    return zipfile, tarfile


def rebinds_a_global():
    global string
    import string


class Namespace:
    import uuid
    import hashlib

    digest = hashlib.sha256


try:
    import tomllib
except ImportError:
    import tomli as tomllib

try:
    import ujson
except ImportError:
    ujson = None

if sys.version_info >= (3, 11):
    import asyncio
else:
    import asyncio

if TYPE_CHECKING:
    import pathlib


def annotated(value: Sequence) -> Optional[int]:
    return value


def deletes_its_import():
    import secrets
    del secrets


def in_a_string_annotation(value: "decimal.Decimal"):
    import decimal
    return value


def imports_beside_locals():
    import colorsys
    return locals()


import zlib

eval("zlib")

import bisect


def an_annotation_hides_an_import():
    bisect: int
    return bisect
