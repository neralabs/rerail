
function Hello() {
  const [data, setData] = useState(0);
  useEffect(() => {
    fetchData.then(setData);
  });
}
