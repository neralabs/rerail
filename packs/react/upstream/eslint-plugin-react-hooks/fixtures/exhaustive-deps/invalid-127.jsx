
function Podcasts({ fetchPodcasts, id }) {
  let [podcasts, setPodcasts] = useState(null);
  useEffect(() => {
    console.log(fetchPodcasts);
    fetchPodcasts?.(id).then(setPodcasts);
  }, [id]);
}
