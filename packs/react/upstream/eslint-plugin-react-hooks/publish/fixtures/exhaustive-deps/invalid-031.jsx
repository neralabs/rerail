
function MyComponent() {
  const local = {};
  const dependencies = [local];
  useEffect(() => {
    console.log(local);
  }, dependencies);
}
