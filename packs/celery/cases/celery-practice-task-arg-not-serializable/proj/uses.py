from proj import reexport
from proj.decorators import aliased, hueyed, packaged, renamed, wrapped
from proj.ops import deploy
from proj.reexport import ship as reship


def through_wrappers(tags):
    wrapped.delay(set(tags))  # caught
    aliased.delay(set(tags))  # caught
    renamed.delay(set(tags))  # caught
    reship.delay({"a"})  # caught
    reexport.ship.delay({"b"})  # caught
    packaged.delay(set(tags))  # caught


def look_alike(tags):
    hueyed.s(set(tags))
    deploy.delay(set(tags))
