
function MyComponent({ fn1, fn2 }) {
  const value = useMemo(fn1);
  const fn = useCallback(fn2);
}
