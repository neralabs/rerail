a, b = 1, 2
c = b if not a else a

a, b = 1, 2
c = b if not a else a  # noqa

a, b = 1, 2
c = b if not a else a  # noqa: SIM212

a, b = 1, 2
c = b if not a else a  # NOQA: SIM212

a, b = 1, 2
c = b if not a else a  # noqa: E501
