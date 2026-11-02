
function Example({ prop }) {
  const foo = useCallback(() => {
    if (prop) {
      foo();
    }
  }, [prop]);
}
