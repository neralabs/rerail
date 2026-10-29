import itertools
x = itertools.starmap(f, zip(a, b))  

import itertools
x = itertools.starmap(f, zip(a, b))    # noqa

import itertools
x = itertools.starmap(f, zip(a, b))    # noqa: RUF058

import itertools
x = itertools.starmap(f, zip(a, b))    # NOQA: RUF058

import itertools
x = itertools.starmap(f, zip(a, b))    # noqa: E501
