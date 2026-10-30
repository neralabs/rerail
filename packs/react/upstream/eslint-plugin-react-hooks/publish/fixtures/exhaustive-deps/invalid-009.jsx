
function MyComponent() {
  const local = /foo/;
  useEffect(() => {
    console.log(local);
  }, []);
}
