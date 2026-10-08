
function MyComponent() {
  const local1 = 42;
  const local2 = '42';
  const local3 = null;
  const local4 = {};
  useEffect(() => {
    console.log(local1);
    console.log(local2);
    console.log(local3);
    console.log(local4);
  }, [local1, local3]);
}
