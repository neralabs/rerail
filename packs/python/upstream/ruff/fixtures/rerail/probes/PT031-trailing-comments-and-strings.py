import pytest


def t():
    with pytest.warns(A):
        foo()
        bar("#x")
    with pytest.warns(A):
        foo()
        bar('#')  # c
    with pytest.warns(A):
        foo()
        bar("""a
#b""")  # c
    with pytest.warns(A):
        foo()
        bar("a\"#")  # c ; x
    with pytest.warns(A):
        foo()
        bar('a\'#') ;  # c
    with pytest.warns(A):
        foo()
        bar(f"{'#'}")  # c
    with pytest.warns(A):
        foo()
        bar(r"\\")  # "x"
    with pytest.warns(A):
        foo()
        bar(b'#', u"#")
    with pytest.warns(A):
        foo()
        x = '''#''' + """#"""  # '''
