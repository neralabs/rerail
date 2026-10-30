
function Podcasts({ fetchPodcasts, id }) {
  let [podcasts, setPodcasts] = useState(null);
  useEffect(() => {
    fetchPodcasts(id).then(setPodcasts);
  }, [id]);
}
