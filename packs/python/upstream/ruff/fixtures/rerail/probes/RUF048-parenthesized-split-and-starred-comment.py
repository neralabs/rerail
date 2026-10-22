import foo
__version__ = "1.2.3"
a = tuple(map(int, (__version__.split)(".")))
b = tuple(map(int, ((foo.__version__).split)(sep=".")))
c = [*  # star then comment
    map(int, __version__.split("."))]
d = map \
    (int, foo.__version__.split \
    ("."))
