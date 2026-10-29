import numpy as np
import builtins

try:
    np.all([1])
except (ValueError, (AttributeError, KeyError)):
    np.alltrue([1])
try:
    np.any([1])
except type(AttributeError):
    np.sometrue([1])
try:
    np.prod([1])
except Exception as AttributeError:
    np.product([1])
try:
    np.round(1)
except [AttributeError][0]:
    np.round_(1)
try:
    np.cumprod(1)
except (builtins.AttributeError,):
    np.cumproduct(1)
