
function MyComponent() {
  const local = {} as string;
  useEffect(() => {
    console.log(local);
  }, []);
}
