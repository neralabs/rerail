
function ctn(afu: { /* comment */ (w: string): number } | undefined): number {
  return afu('hello');
}
      