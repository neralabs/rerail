
function MyComponent() {
  const local1 = {};
  useCallback(() => {
    const local1 = {};
    console.log(local1);
  }, [local1]);
}
