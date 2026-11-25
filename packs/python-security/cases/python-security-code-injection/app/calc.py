import ast

from django.http import HttpResponse

OPS = {"+", "-", "*", "/"}


def validate_expr(expr):
    # Named like a check, but it evaluates the expression itself.
    return eval(expr, {"__builtins__": {}}, {})  # caught


def is_arith(expr):
    return all(isinstance(n, (ast.Expression, ast.BinOp, ast.Constant, ast.operator)) for n in ast.walk(ast.parse(expr)))


def safe_eval(expr):
    return eval(expr, {"__builtins__": {}}, {})  # caught


def calc(request):
    expr = request.GET["expr"]
    validate_expr(expr)
    return HttpResponse(str(eval(expr)))  # caught


def calc_safe_name(request):
    expr = request.GET["expr"]
    return HttpResponse(str(safe_eval(expr)))


def calc_checked(request):
    expr = request.GET["expr"]
    if not is_arith(expr):
        return HttpResponse(status=400)
    return HttpResponse(str(eval(expr, {"__builtins__": {}}, {})))
