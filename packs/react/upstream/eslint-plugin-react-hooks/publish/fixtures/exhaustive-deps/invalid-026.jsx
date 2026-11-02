
function MyComponent({ history }) {
  useEffect(() => {
    return [
      history?.foo
    ];
  }, []);
}
