"""Record original source tests and preserve every returned DTO field.

Use the unchanged recorder's CLI. This adds sourceResult alongside its legacy,
normalized findings; it does not replace assertions or execute Rerail.
"""
from functools import wraps
from dto import canonical


def main():
    from migration import ROOT  # Also locates the existing source recorder.
    import data_layer_record as recording
    original_recorder = recording.Recorder

    class Recorder(original_recorder):
        def pytest_configure(self):
            super().pytest_configure()
            adapters = {(check, entry): kind for check, entry, kind in
                        (*recording.ADAPTERS, *recording.batch_two.ADAPTERS)}
            adapters.update({('data_layer_compat_aliases', entry): 'facade_aliases'
                for entry in ['lint_file', 'lint_file_content']})
            for module, entry, _ in self.originals:
                check = module.__name__.removeprefix('scripts.lint_')
                kind = adapters.get((check, entry))
                if kind is None:
                    continue
                wrapped = getattr(module, entry)

                def capture(module, entry, check, kind, wrapped):
                    @wraps(wrapped)
                    def run(*args, **kwargs):
                        if self.item is None or self.depth:
                            return wrapped(*args, **kwargs)
                        first = len(self.events)
                        try:
                            if kind == 'facade_aliases':
                                from aliases_port import snapshot
                                _, _, root = snapshot(module, entry, args, kwargs, self.item, self.root)
                            else:
                                _, _, root = (recording.batch_two.snapshot(module, entry, args, kwargs)
                                          if kind == 'batch_two' else recording.snapshot(module, kind, args, kwargs))
                        except Exception:
                            root = None
                        result = wrapped(*args, **kwargs)
                        if len(self.events) != first + 1:
                            raise AssertionError('Original call inventory changed during lossless capture')
                        event = self.events[first]
                        if (event['check'], event['entry']) != (check, entry):
                            raise AssertionError('Original call identity changed during lossless capture')
                        event['sourceResult'] = canonical(result, root)
                        return result
                    return run

                setattr(module, entry, capture(module, entry, check, kind, wrapped))

    recording.Recorder = Recorder
    recording.main()


if __name__ == '__main__':
    main()
