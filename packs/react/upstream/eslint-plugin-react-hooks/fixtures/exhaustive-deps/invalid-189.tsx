
function useCustomCallback(callback, deps) {
  return useCallback(callback as any, deps)
}
