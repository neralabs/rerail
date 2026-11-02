
function MyComponent() {
  const local = {id: 42};
  useEffect(() => {
    console.log(local);
  }, [local.id]);
}
