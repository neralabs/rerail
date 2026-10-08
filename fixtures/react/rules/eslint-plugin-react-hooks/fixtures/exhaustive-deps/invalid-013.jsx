
function MyComponent() {
  const local = someFunc();
  useEffect(() => {
    if (true) {
      console.log(local);
    }
  }, []);
}
