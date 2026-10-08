
function MyComponent() {
  const local1 = {};
  const local2 = {};
  useEffect(() => {
    console.log(local1);
    console.log(local2);
  }, [local1]);
}
