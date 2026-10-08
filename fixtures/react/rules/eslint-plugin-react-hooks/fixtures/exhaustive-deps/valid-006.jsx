
function MyComponent() {
  const local1 = someFunc();
  function MyNestedComponent() {
    const local2 = someFunc();
    useCallback(() => {
      console.log(local1);
      console.log(local2);
    }, [local2]);
  }
}
