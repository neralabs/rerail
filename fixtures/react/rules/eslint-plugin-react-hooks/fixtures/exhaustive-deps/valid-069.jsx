
function MyComponent() {
  const local1 = 42;
  const local2 = '42';
  const local3 = null;
  useEffect(() => {
    console.log(local1);
    console.log(local2);
    console.log(local3);
  }, []);
}
