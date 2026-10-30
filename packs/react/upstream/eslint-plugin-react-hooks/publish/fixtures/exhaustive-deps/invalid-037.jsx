
function MyComponent({ items }) {
  useEffect(() => {
    console.log(items[0]);
  }, [items[0]]);
}
