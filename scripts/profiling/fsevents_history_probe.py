"""FSEvents history probe: does a replay since a cursor name a file changed after the cursor?

Each trial, in a fresh directory on the volume of `root`:
  1. write X (it enters fseventsd's buffer with some id a);
  2. a live stream from now: write a cookie, wait for its event; the cookie's id is the cursor c (as the engine's sync);
     stop the stream (as a process ends);
  3. change X again; create Y (new, the control); optionally sleep `gap`;
  4. a new stream since c (FullHistory or not): write cookie 2, wait for HistoryDone and cookie 2;
     record whether X and Y came, with their ids against c.
Prints one line per trial and a summary per variant.
"""
import ctypes as C, ctypes.util, os, sys, time, threading, json

CS = C.cdll.LoadLibrary('/System/Library/Frameworks/CoreServices.framework/CoreServices')
CF = C.cdll.LoadLibrary('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
SYS = C.cdll.LoadLibrary('/usr/lib/libSystem.dylib')

CB = C.CFUNCTYPE(None, C.c_void_p, C.c_void_p, C.c_size_t, C.c_void_p, C.POINTER(C.c_uint32), C.POINTER(C.c_uint64))
CF.CFStringCreateWithCString.restype = C.c_void_p
CF.CFStringCreateWithCString.argtypes = [C.c_void_p, C.c_char_p, C.c_uint32]
CF.CFArrayCreate.restype = C.c_void_p
CF.CFArrayCreate.argtypes = [C.c_void_p, C.POINTER(C.c_void_p), C.c_long, C.c_void_p]
CS.FSEventStreamCreate.restype = C.c_void_p
CS.FSEventStreamCreate.argtypes = [C.c_void_p, CB, C.c_void_p, C.c_void_p, C.c_uint64, C.c_double, C.c_uint32]
for f in ('FSEventStreamStart',):
    getattr(CS, f).restype = C.c_ubyte
    getattr(CS, f).argtypes = [C.c_void_p]
for f in ('FSEventStreamStop', 'FSEventStreamInvalidate', 'FSEventStreamRelease'):
    getattr(CS, f).restype = None
    getattr(CS, f).argtypes = [C.c_void_p]
CS.FSEventStreamSetDispatchQueue.argtypes = [C.c_void_p, C.c_void_p]
CS.FSEventsGetCurrentEventId.restype = C.c_uint64
SYS.dispatch_queue_create.restype = C.c_void_p
SYS.dispatch_queue_create.argtypes = [C.c_char_p, C.c_void_p]
ARRAY_CB = C.addressof(C.c_void_p.in_dll(CF, 'kCFTypeArrayCallBacks'))

NO_DEFER, FILE_EVENTS, FULL_HISTORY = 0x2, 0x10, 0x80
HISTORY_DONE = 0x10
QUEUE = SYS.dispatch_queue_create(b'probe', None)


class Stream:
    def __init__(self, path, since, flags):
        self.events = []  # (id, path, flags, phase) phase: 'h' before HistoryDone, 'l' after
        self.done = False
        self.cv = threading.Condition()

        def cb(_s, _i, n, paths, fl, ids):
            pp = C.cast(paths, C.POINTER(C.c_char_p))
            with self.cv:
                for k in range(n):
                    if fl[k] & HISTORY_DONE:
                        self.done = True
                        continue
                    self.events.append((ids[k], pp[k].decode(), fl[k], 'l' if self.done else 'h'))
                self.cv.notify_all()

        self.cb = CB(cb)
        s = CF.CFStringCreateWithCString(None, path.encode(), 0x08000100)
        arr = (C.c_void_p * 1)(s)
        paths = CF.CFArrayCreate(None, arr, 1, ARRAY_CB)
        self.s = CS.FSEventStreamCreate(None, self.cb, None, paths, since, 0.0, NO_DEFER | FILE_EVENTS | flags)
        CS.FSEventStreamSetDispatchQueue(self.s, QUEUE)
        assert CS.FSEventStreamStart(self.s)

    def wait(self, pred, timeout=5.0):
        end = time.time() + timeout
        with self.cv:
            while not pred():
                left = end - time.time()
                if left <= 0:
                    return False
                self.cv.wait(left)
        return True

    def cookie(self, path, history=True):
        open(path, 'w').close()
        ok = self.wait(lambda: (self.done or not history) and any(p == path for _, p, _, _ in self.events))
        ids = [i for i, p, _, _ in self.events if p == path]
        return ok, (ids[0] if ids else None)

    def close(self):
        CS.FSEventStreamStop(self.s)
        CS.FSEventStreamInvalidate(self.s)
        CS.FSEventStreamRelease(self.s)


SINCE_NOW = 0xFFFFFFFFFFFFFFFF


def trial(base, n, flags, gap, rewrite_first):
    d = os.path.realpath(os.path.join(base, f't{n}-{flags}-{gap}-{rewrite_first}'))
    os.makedirs(d)
    x, y = os.path.join(d, 'x.py'), os.path.join(d, 'y.py')
    with open(x, 'w') as f:
        f.write('a = 1\n')
    if rewrite_first:  # the earlier run's file existed long before: written, then the cursor much later
        time.sleep(0.2)
    live = Stream(d, SINCE_NOW, 0)
    ok, cursor = live.cookie(os.path.join(d, 'c1'), history=False)
    live.close()
    if not ok:
        return {'n': n, 'err': 'no live cookie'}
    with open(x, 'w') as f:  # the edit, after the cursor
        f.write('a = 2  # edited\n')
    with open(y, 'w') as f:
        f.write('b = 1\n')
    if gap:
        time.sleep(gap)
    m = os.path.join(d, 'premarker')
    open(m, 'w').close()  # written before the stream: it must come in the history (fil#51's check)
    t = time.time()
    s = Stream(d, cursor, flags)
    ok, c2 = s.cookie(os.path.join(d, 'c2'))
    took = time.time() - t
    at_cookie = len(s.events)
    time.sleep(2.0)  # later events on the same stream: late, or never
    s.close()
    ev = s.events
    time.sleep(0.5)
    s3 = Stream(d, cursor, flags)  # a replay from the same cursor, 2.5 s later
    ok3, _ = s3.cookie(os.path.join(d, 'c3'))
    s3.close()

    def seen(p, events, upto=None):
        evs = events if upto is None else events[:upto]
        return ''.join(sorted({ph for i, q, _, ph in evs if q == p})) or '-'

    return {'n': n, 'ok': ok, 'took': round(took, 3),
            'x': seen(x, ev, at_cookie), 'y': seen(y, ev, at_cookie), 'm': seen(m, ev, at_cookie), 'c2': seen(os.path.join(d, 'c2'), ev, at_cookie),
            'x_late': seen(x, ev[at_cookie:]), 'y_late': seen(y, ev[at_cookie:]),
            'x_again': seen(x, s3.events), 'y_again': seen(y, s3.events), 'ok3': ok3}


def main():
    base = sys.argv[1]
    trials = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    os.makedirs(base, exist_ok=True)
    out = []
    for flags in (0, FULL_HISTORY):
        for gap in (0.0, 0.5):
            for rf in (False, True):
                rs = [trial(base, n, flags, gap, rf) for n in range(trials)]
                miss = [r for r in rs if r.get('x') == '-' or r.get('y') == '-']
                mh = sum(r.get('m', '-') != 'h' for r in rs)
                caught = sum(r.get('m', '-') != 'h' for r in miss)
                took = sorted(r.get('took', 0) for r in rs)[len(rs) // 2]
                print(f"flags={'full' if flags else 'none'} gap={gap} earlier_write={rf}: missed at the cookie {len(miss)}/{trials}; "
                      f"pre-marker not in history {mh}/{trials} (in {caught} of the misses); median replay {took:.3f} s", flush=True)
                for r in miss:
                    print('   ', json.dumps(r), flush=True)
                out += rs


main()
