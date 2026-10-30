
      class Hello extends React.Component {
        componentDidMount() {
          someClass.onSomeEvent((data) => this.setState({data: data}));
        }
      }
    