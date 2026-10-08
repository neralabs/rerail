
function MyComponent() {
  const local1 = someFunc();
  {
    const local2 = someFunc();
    useEffect(() => {
      console.log(local1);
      console.log(local2);
    }, []);
  }
}
