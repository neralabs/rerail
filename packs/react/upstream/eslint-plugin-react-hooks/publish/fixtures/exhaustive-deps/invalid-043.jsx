
function MyComponent() {
  const local = {id: 42};
  const fn = useCallback(() => {
    console.log(local);
  }, [local.id, local]);
}
