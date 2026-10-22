d = {}
k = "a"
if k in d and d[k]:  
    pass

d = {}
k = "a"
if k in d and d[k]:    # noqa
    pass

d = {}
k = "a"
if k in d and d[k]:    # noqa: RUF019
    pass

d = {}
k = "a"
if k in d and d[k]:    # NOQA: RUF019
    pass

d = {}
k = "a"
if k in d and d[k]:    # noqa: E501
    pass
