
function Example() {
  const foo = useCallback(() => {
    foo();
  }, []);
}
