
function MyComponent() {
  const local = {};
  useEffect(debounce(() => {
    console.log(local);
  }, delay), []);
}
