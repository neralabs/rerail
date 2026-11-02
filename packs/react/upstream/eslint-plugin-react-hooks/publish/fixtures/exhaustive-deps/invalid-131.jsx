
function Hello({ country }) {
  const [data, setData] = useState(0);
  useEffect(() => {
    fetchData(country).then(setData);
  });
}
