
function MyComponent() {
  const local = {};
  useEffect(() => {
    console.log(local);
  }, [a && local]);
}
