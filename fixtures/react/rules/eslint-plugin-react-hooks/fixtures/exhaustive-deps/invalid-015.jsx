
function MyComponent() {
  const local = {};
  useEffect(() => {
    function inner() {
      console.log(local);
    }
    inner();
  }, []);
}
