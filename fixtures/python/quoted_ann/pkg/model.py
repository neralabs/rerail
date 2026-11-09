from __future__ import annotations
from typing import Optional


class Node:
    def child(self) -> "Node":
        return self

    def parent(self) -> Optional["Node"]:
        return None

    def label(self) -> str:
        return "n"


def make() -> "Node":
    return Node()


def first(xs: "list[Node]") -> "Node | None":
    return None
