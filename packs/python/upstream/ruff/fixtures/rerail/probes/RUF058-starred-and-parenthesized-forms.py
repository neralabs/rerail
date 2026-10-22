import itertools
from itertools import starmap as sm
import itertools as it
itertools.starmap(*fs, zip(a))
sm(f, zip(*a, b))
sm(f, zip(*a, *b))
sm(f, zip())
it.starmap(f, (zip(a)))
(it.starmap)(f, zip(a))
sm(f, *zip(a))
sm(f, zip(a), )
sm(f, zip(a), **kw)
sm(*fs)
sm(f, zip(x for x in y))
sm(f, zip(a)(b))
sm(f, zip(a, strict=True))
sm(f, zip(a, **kw))
sm(f, (zip)(a))
sm(f, zip(a) if c else zip(b))
sm(f,
   # comment
   zip(a, b)  # trailing
)
