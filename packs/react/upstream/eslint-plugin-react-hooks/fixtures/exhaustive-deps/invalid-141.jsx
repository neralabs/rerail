
function MyComponent() {
  const local = {};
  const myEffect = debounce(() => {
    console.log(local);
  }, delay);
  useEffect(myEffect, []);
}
