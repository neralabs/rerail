__all__ = ["a"]
__all__.append("b")  

__all__ = ["a"]
__all__.append("b")    # noqa

__all__ = ["a"]
__all__.append("b")    # noqa: PYI056

__all__ = ["a"]
__all__.append("b")    # NOQA: PYI056

__all__ = ["a"]
__all__.append("b")    # noqa: E501
