"""A nested def is scored on its own; a lambda's decisions count toward the enclosing def."""
import functools


def outer_with_complex_inner(values):
    def inner(value):
        if value == 1:
            return "one"
        if value == 2:
            return "two"
        if value == 3:
            return "three"
        if value == 4:
            return "four"
        if value == 5:
            return "five"
        if value == 6:
            return "six"
        return "many"

    if values:
        return [inner(value) for value in values]
    return []


def outer_with_lambdas(values):
    if not values:
        return []
    if len(values) > 10:
        values = values[:10]
    if len(values) > 5:
        values = values[:5]
    first = sorted(values, key=lambda value: 0 if value is None else value)
    second = map(lambda value: value and value.strip(), first)
    third = filter(lambda value: value or None, second)
    return list(third)


class Dispatcher:
    def method_over_limit(self, event):
        if event == "a":
            return 1
        if event == "b":
            return 2
        if event == "c":
            return 3
        for handler in self.handlers:
            if handler:
                return handler(event)
        while self.pending:
            self.pending.pop()
        return 0

    def simple_method(self):
        return self.handlers


@functools.cache
def decorated_over_limit(code):
    if code == 1:
        return "one"
    if code == 2:
        return "two"
    if code == 3:
        return "three"
    if code == 4:
        return "four"
    if code == 5:
        return "five"
    if code == 6:
        return "six"
    return "many"
