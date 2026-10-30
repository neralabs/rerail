
        var Hello = React.createClass({
          onClick: function() {
            var nextValueNotUsed = this.state.value + 1
            var nextValue = 2
            this.setState({value: nextValue})
          }
        });
      