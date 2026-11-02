
      var Hello = createReactClass({
        componentDidMount: function() {
          someClass.onSomeEvent((data) => this.setState({data: data}));
        }
      });
    