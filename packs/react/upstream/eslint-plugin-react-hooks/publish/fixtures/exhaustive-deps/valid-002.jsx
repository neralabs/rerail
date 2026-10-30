
function MyComponent() {
  const local = someFunc();
  useEffect(() => {
    console.log(local);
  }, [local]);
}
