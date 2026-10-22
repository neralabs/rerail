import os

p = "a/b"
(p.split)(os.sep)
((p).split)(sep=(os.sep))
p.split(  # leading
    sep=os.sep  # trailing
)
q = p \
    .split(os.sep)
p.split(os.sep, 1)
p.split(sep=os.sep, maxsplit=1)
p.split(*[os.sep])
