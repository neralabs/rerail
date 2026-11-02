
function MyComponent() {
  const local1 = {};
  const local2 = {};
  useMemo(() => {
    console.log(local1);
  }, [local1, local2]);
}
