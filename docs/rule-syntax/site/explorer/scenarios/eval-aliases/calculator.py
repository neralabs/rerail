import ast
import builtins as b
from builtins import eval as evaluate


def unsafe(expression):
    return eval(expression)


def also_unsafe(expression):
    return (eval)(expression)


def aliased(expression):
    return evaluate(expression)


def through_module(expression):
    return b.eval(expression)


def safe(expression):
    # Parses literals only: numbers, strings, tuples, lists, dicts.
    return ast.literal_eval(expression)
