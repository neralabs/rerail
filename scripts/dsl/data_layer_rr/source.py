"""Record original source tests and preserve every returned DTO field.

Use the unchanged recorder's CLI. This adds sourceResult alongside its legacy,
normalized findings; it does not replace assertions or execute Rerail.
"""
from functools import wraps
from dto import canonical


def main():
    from migration import ROOT  # Also locates the existing source recorder.
    import data_layer_record as recording
    recording.ADAPTERS += (('hnsw_architecture', 'find_hnsw_architecture_findings', 'files'),
                           ('backend_type_strings', 'should_skip_file', 'backend_helper'),
                           ('backend_type_strings', 'find_python_files', 'backend_helper'),
                           ('error_handling', 'scan_file_content', 'error_file'),
                           ('connection_caches', 'find_violations', 'connection_targets'),
                           ('data_layer_facade_purity', 'lint_targets', 'facade_targets'))
    original_snapshot = recording.snapshot
    def input_snapshot(module, kind, args, kwargs):
        if kind == 'error_file':
            from error_handling_port import snapshot
            return snapshot(module, 'scan_file_content', args, kwargs)
        if kind == 'connection_targets':
            from connection_caches_port import snapshot
            return snapshot(module, 'find_violations', args, kwargs)
        if kind == 'facade_targets':
            from facade_purity_port import snapshot
            return snapshot(module, 'lint_targets', args, kwargs)
        return original_snapshot(module, kind, args, kwargs)
    recording.snapshot = input_snapshot
    original_normalize = recording.normalize
    def result_snapshot(check, entry, result, root, files):
        if check == 'backend_type_strings' and entry in {'should_skip_file', 'find_python_files'}:
            return []  # Helpers return values, not detector findings; sourceResult keeps the full value.
        if check == 'error_handling':
            from error_handling_port import normalize
            return normalize(result, root)
        if check == 'connection_caches' and entry == 'find_violations':
            from connection_caches_port import normalize
            return normalize(result, root)
        if check == 'data_layer_facade_purity' and entry == 'lint_targets':
            return recording.batch_five.normalize(check, result.violations, root, files)
        return original_normalize(check, entry, result, root, files)
    recording.normalize = result_snapshot
    original_recorder = recording.Recorder

    class Recorder(original_recorder):
        def pytest_configure(self):
            current_snapshot = recording.snapshot
            def case_snapshot(module, kind, args, kwargs):
                if kind == 'backend_helper':
                    return [], dict(include=[], exclude=[]), self.item.funcargs.get('tmp_path') if self.item else None
                return current_snapshot(module, kind, args, kwargs)
            recording.snapshot = case_snapshot
            super().pytest_configure()
            adapters = {(check, entry): kind for check, entry, kind in
                        (*recording.ADAPTERS, *recording.batch_two.ADAPTERS)}
            adapters[('sync_async_signatures', 'analyze_file')] = 'file'
            adapters[('data_layer_facade_purity', 'lint_file')] = 'file'
            adapters.update({('data_layer_compat_aliases', entry): 'facade_aliases'
                for entry in ['lint_file', 'lint_file_content']})
            for module, entry, _ in self.originals:
                backend_checker = isinstance(module, type) and module.__name__ == 'BackendCouplingChecker'
                check = 'backend_coupling' if backend_checker else module.__name__.removeprefix('scripts.lint_')
                kind = 'backend_checker' if backend_checker and entry == 'check_file' else adapters.get((check, entry))
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
                            if kind == 'backend_checker':
                                from coupling_port import snapshot
                                _, _, root = snapshot(args[2], args[3])
                            elif kind == 'facade_aliases':
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
                        recorded_entry = 'BackendCouplingChecker.check_file' if kind == 'backend_checker' else entry
                        if (event['check'], event['entry']) != (check, recorded_entry):
                            raise AssertionError('Original call identity changed during lossless capture')
                        event['sourceResult'] = canonical(result, root)
                        if kind == 'backend_helper':
                            event['observationKind'] = 'helper-value'
                            if event.get('retained_reason') == 'No readable source files (file discovery or I/O contract)':
                                event.pop('retained_reason')
                        if check == 'sync_async_signatures':
                            for finding in event.get('findings', []):
                                finding['message'] = canonical(finding['message'], root)
                        return result
                    return run

                setattr(module, entry, capture(module, entry, check, kind, wrapped))

            module = next((owner for owner, _, _ in self.originals
                           if getattr(owner, '__name__', '') == 'scripts.lint_unresolved_forward_refs'), None)
            if module is not None:
                owner, original = module.AnnotationUsageFinder, module.AnnotationUsageFinder.analyze
                @wraps(original)
                def analyze(instance, tree):
                    if self.item is None or self.depth:
                        return original(instance, tree)
                    from unresolved_port import supplied_snapshot, analyzed_state
                    files, scope, root, syntax = supplied_snapshot(instance, tree)
                    result = original(instance, tree)
                    findings = recording.batch_two.normalize('unresolved_forward_refs', instance.findings, root, files)
                    self.events.append(dict(test=self.item.nodeid, check='unresolved_forward_refs',
                        entry='AnnotationUsageFinder.analyze', call=1, files=files, scope=scope,
                        input_mode='supplied_ast', suppliedAst=syntax, findings=findings,
                        sourceResult=canonical(analyzed_state(instance, result), root), observationKind='instance-state'))
                    return result
                self.originals.append((owner, 'analyze', original))
                owner.analyze = analyze

    recording.Recorder = Recorder
    recording.main()


if __name__ == '__main__':
    main()
