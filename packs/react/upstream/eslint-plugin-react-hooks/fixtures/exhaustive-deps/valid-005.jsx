
function MyComponent() {
  const local1 = someFunc();
  {
    const local2 = someFunc();
    useCallback(() => {
      console.log(local1);
      console.log(local2);
    }, [local1, local2]);
  }
}
