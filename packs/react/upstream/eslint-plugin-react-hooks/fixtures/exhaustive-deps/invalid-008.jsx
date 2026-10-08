
function MyComponent() {
  let local = 42;
  useEffect(() => {
    console.log(local);
  }, []);
}
