from __future__ import annotations


def forward(x: Later, y: Missing) -> Later:
    return x, y


class Model:
    field: Later
    other: Missing = None

    def method(self) -> Model:
        return self


class Later:
    pass


annotated_by_a_walrus: (bound_in_the_annotation := 1)
print(bound_in_the_annotation)


class WalrusInClass:
    field: (class_walrus := 1)
    read = class_walrus
