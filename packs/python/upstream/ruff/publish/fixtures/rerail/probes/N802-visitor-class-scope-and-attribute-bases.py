import ast
import http.server


class Outer:
    class Inner(ast.NodeVisitor):
        pass

    class Handler(http.server.BaseHTTPRequestHandler):
        pass

    class V2(Inner):
        def visit_Nested(self, node): ...


class V(Outer.Inner):
    def visit_Attr(self, node): ...


class H(Outer.Handler):
    def do_GET(self): ...


class H2(Outer.Handler):
    def do_POST(self): ...

    if True:
        def do_PUT(self): ...

    def DoIt(self): ...
