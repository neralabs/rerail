
function MyComponent({ history }) {
  useEffect(() => {
    return history.listen();
  }, [history]);
}
