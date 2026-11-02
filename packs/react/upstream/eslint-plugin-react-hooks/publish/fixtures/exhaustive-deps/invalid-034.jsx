
function MyComponent() {
  const local = {};
  useEffect(() => {
    console.log(local);
  }, [computeCacheKey(local)]);
}
