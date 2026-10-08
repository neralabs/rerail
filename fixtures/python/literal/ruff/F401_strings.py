"""Imports read only from string type definitions, and imports named by strings that are none."""

import typing
from typing import TYPE_CHECKING, Annotated, List, Literal, NamedTuple, TypedDict, TypeVar, cast

import collections  # read in a string annotation
import decimal  # named only by a Literal string: unused
import fractions  # named only by Annotated metadata: unused
import ipaddress  # read in a cast's type
import json  # named only by a non-typing call's string: unused
import numbers  # read in a TypeVar bound
import pathlib  # read in a generic's slice at runtime
import queue  # read in a nested string
import random  # read in a NamedTuple field type
import shelve  # read in a TypedDict value type
import socket  # read in an invalid annotation: Ruff reads nothing there, unused
import statistics  # named in an f-string: unused
import string  # read in an Annotated first element
import textwrap  # read in a return annotation
import uuid  # read by `typing.cast`
import zlib  # named only in a comment-like string: unused

if TYPE_CHECKING:
    import heapq  # read in a type-checking-only string annotation


def first(value: "collections.OrderedDict[str, int]") -> "textwrap.TextWrapper":
    return value


def second(value: Literal["decimal"]) -> None:
    return None


def third(value: Annotated["string.Template", "fractions"]) -> None:
    return None


def fourth(value: object) -> None:
    cast("ipaddress.IPv4Address", value)
    typing.cast("uuid.UUID", value)
    print("json")
    print(f"{'statistics'}")


T = TypeVar("T", bound="numbers.Number")
Paths = List["pathlib.Path"]
Nested = List["List['queue.Queue']"]
Point = NamedTuple("Point", [("x", "random.Random")])
Movie = TypedDict("Movie", {"name": "shelve.Shelf"})


def fifth(value: "socket.socket[") -> None:
    return None


def sixth(value: "heapq") -> None:
    return None


note = "zlib"
