
function Podcasts({ id }) {
  let [podcasts, setPodcasts] = useState(null);
  useEffect(() => {
    function doFetch({ fetchPodcasts }) {
      fetchPodcasts(id).then(setPodcasts);
    }
    doFetch({ fetchPodcasts: API.fetchPodcasts });
  }, [id]);
}
