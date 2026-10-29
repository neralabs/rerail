class A:
    def __str__(self) -> str: ...  

class A:
    def __str__(self) -> str: ...    # noqa

class A:
    def __str__(self) -> str: ...    # noqa: PYI029

class A:
    def __str__(self) -> str: ...    # NOQA: PYI029

class A:
    def __str__(self) -> str: ...    # noqa: E501
