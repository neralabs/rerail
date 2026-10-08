from __future__ import annotations

import os
import os
import sys
print(sys)
import sys
import json
json = None
import re as regex
from re import compile as regex
import xml.etree
import xml.etree
import xml.dom
import email as mail
import email.parser as mail
from collections import OrderedDict
from collections import OrderedDict as OrderedDict
import typing
from typing import overload, TYPE_CHECKING

if TYPE_CHECKING:
    import pathlib
import pathlib

try:
    import tomllib
except ImportError:
    import tomli as tomllib

if sys.platform == "win32":
    import ntpath as pathmod
else:
    import posixpath as pathmod

import csv
if sys.argv:
    import csv


def duplicate():
    pass


def duplicate():
    pass


class Duplicate:
    pass


class Duplicate:
    pass


value = 1
value = 2


def uses_its_own():
    pass


uses_its_own()


def uses_its_own():
    pass


@overload
def overloaded(x: int) -> int: ...


@overload
def overloaded(x: str) -> str: ...


def overloaded(x):
    return x


def shadows_a_module_import():
    import glob
    glob = 1
    import fnmatch
    return glob, fnmatch


import fnmatch
import glob
import heapq
import heapq as heapq


def local_shadows_heapq():
    import heapq
    return heapq


def parameter(os):
    return os


class Attributes:
    import shutil

    def shutil(self):
        pass

    json = 1


def loop_over_it():
    for tomllib in []:
        pass


import string
del string
import string

_ = 1


def _():
    pass


print(os, regex, xml, mail, OrderedDict, typing, pathlib, tomllib, pathmod, csv, duplicate, Duplicate, value)
print(uses_its_own, overloaded, heapq, parameter, Attributes, loop_over_it, string, fnmatch, glob)
