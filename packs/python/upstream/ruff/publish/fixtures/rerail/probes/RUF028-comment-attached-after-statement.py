def f():
    x = [  # fmt: off
        1
    ]
    # fmt: on
    y = 1


def g():
    x = (  # fmt: off
        1
    )
    # fmt: on
    y = 1


x = (  # fmt: off
    1
)  # fmt: skip
x = a + (  # fmt: off
    b
)  # fmt: skip
x = [  # fmt: off
    1
]
# fmt: on
