
      class Hello extends React.Component {
        componentDidMount() {
          someClass.onSomeEvent(function(data) {
            this.setState({
              data: data
            });
          })
        }
      }
    