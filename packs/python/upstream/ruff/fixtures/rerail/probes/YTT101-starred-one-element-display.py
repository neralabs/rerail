import sys
from sys import version

a = [*sys.version[:3]]
b = {*version[:3]}
c = [*  # c
     sys.version[:3:]]
d = [*sys.version[:3], 1]
e = [*sys.version[1:3]]
