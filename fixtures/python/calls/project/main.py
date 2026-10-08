"""Runs every call site of the workspace (rerail-fixtures/python/calls/generate.py traces it)."""
import pkg
from pkg import aliases, asyncs, decorators, dynamic, models, nested, util
from pkg.util import helper as h


def main():
    util.run_all()
    h(1)
    pkg.exported_helper(2)
    pkg.package_level()
    nested.outer(3)
    models.exercise()
    decorators.exercise()
    dynamic.exercise()
    aliases.exercise()
    asyncs.exercise()


main()
