
function withFetch(fetchPodcasts) {
  return function Podcasts({ id }) {
    let [podcasts, setPodcasts] = useState(null);
    useEffect(() => {
      fetchPodcasts(id).then(setPodcasts);
    }, [id]);
  }
}
