import itertools

# A comment or backslash in the argument list is not an argument.
zip(a, \
    )
zip(a,  # c
    )
zip(a, itertools.repeat(1, \
                        ))
zip(a, itertools.repeat(1, times  # c
                        =None))
zip(a, itertools.repeat(1, \
                        None))
zip(a, b \
    )

# tree-sitter reads a starred call first in a list display as a call of `*zip`.
[*zip(a, b)][0]
[*zip(a, b, strict=True)]
[*zip(a)]
