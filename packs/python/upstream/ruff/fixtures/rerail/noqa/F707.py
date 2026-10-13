try:
    pass
except:
    pass
except ValueError:
    pass

try:
    pass
except:  # noqa
    pass
except ValueError:
    pass

try:
    pass
except:  # noqa: F707
    pass
except ValueError:
    pass

try:
    pass
except:  # NOQA: F707
    pass
except ValueError:
    pass

try:
    pass
except:  # noqa: E501
    pass
except ValueError:
    pass
