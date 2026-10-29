
function MyComponent() {
  const local = someFunc();
  useEffect(() => {
    console.log(local);
    console.log(local);
  }, [local]);
}
