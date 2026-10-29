d = {}
if 'a' in d.keys():
    pass

d = {}
if 'a' in d.keys():  # noqa
    pass

d = {}
if 'a' in d.keys():  # noqa: SIM118
    pass

d = {}
if 'a' in d.keys():  # NOQA: SIM118
    pass

d = {}
if 'a' in d.keys():  # noqa: E501
    pass
