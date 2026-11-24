# Event buses, chat frameworks and socket servers have their own on_event: left alone.
import socketio
from slack_bolt import App


class Bus:
    def __init__(self) -> None:
        self.handlers = {}

    def on_event(self, name, handler=None):
        self.handlers[name] = handler

    def add_event_handler(self, name, handler):
        self.handlers[name] = handler

    def wire(self, name, handler):
        self.on_event(name, handler)


bus = Bus()
bus.on_event("startup", print)
bus.add_event_handler("shutdown", print)


def handler(event) -> None: ...


class Plugin:
    def setup(self, events):
        events.on_event(handler)
        events.on_event("message", handler)
        self.on_event(self.name)


slack = App(token="x")


@slack.event("app_mention")
def mention(event, say) -> None: ...


sio = socketio.AsyncServer()


@sio.on("connect")
async def connect(sid, environ) -> None: ...
