
function MyComponent() {
  const local = {};
  useEffect(() => {
    console.log(local);
  }, [local, local]);
}
