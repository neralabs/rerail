
function MyComponent() {
  const local = {};
  useEffect(() => {
    try {
      console.log(local);
    } finally {}
  }, []);
}
