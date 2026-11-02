
function MyComponent({foo}) {
  return useMemo(() => foo, [foo])
}
