import os  # noqa
import sys  # noqa: F401
import re  # noqa:F401,E501
import json  # NOQA
import csv  #noqa
import abc  # noqa : F401
import ast  # noqa: F401 because it is used elsewhere
import bdb  #noqaF401
import cgi  # noqa:
import cmd  # noqa: f401
import dis  # a comment # noqa: F401
import dbm  # noqa: E501
import enum  # noqa:F401F841
import glob  # noqa: F401,,E501
import gzip  # noqa: F401#another comment
import heapq  # noqa:E501 F401
import hmac  # noqa: F401, TCH002
import html  # noqa—F401
from io import *  # noqa: F403


def f():
    l = 1  # noqa: E741
    I = 2  # noqa: F841
    O = 3  # noqa: F841, E741
    unused = 4  # noqa
    x = f"plain"  # noqa: F541
    y = "\d"  # noqa: W605
    if 1 == x:  # noqa: SIM300
        pass
    return y
