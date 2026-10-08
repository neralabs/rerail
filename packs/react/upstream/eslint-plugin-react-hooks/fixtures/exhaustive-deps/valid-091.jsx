
function App() {
  const [query, setQuery] = useState('react');
  const [state, setState] = useState(null);
  useEffect(() => {
    let ignore = false;
    fetchSomething();
    async function fetchSomething() {
      const result = await (await fetch('http://hn.algolia.com/api/v1/search?query=' + query)).json();
      if (!ignore) setState(result);
    }
    return () => { ignore = true; };
  }, [query]);
  return (
    <>
      <input value={query} onChange={e => setQuery(e.target.value)} />
      {JSON.stringify(state)}
    </>
  );
}
