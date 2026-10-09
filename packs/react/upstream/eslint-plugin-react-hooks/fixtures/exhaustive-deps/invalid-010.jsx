
function MyComponent(props) {
  const value = useMemo(() => { return 2*2; });
  const fn = useCallback(() => { alert('foo'); });
}
