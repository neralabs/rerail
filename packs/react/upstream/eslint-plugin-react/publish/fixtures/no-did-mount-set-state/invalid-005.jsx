
      var Hello = createReactClass({
        componentDidMount: function() {
          someClass.onSomeEvent(function(data) {
            this.setState({
              data: data
            });
          })
        }
      });
    