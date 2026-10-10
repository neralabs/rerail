
function foo(bar: { /* comment */ (s: string): number } | undefined): number {
  return bar('hello');
}
      