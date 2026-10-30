
        function nextState(state) {
          return {value: state.value + 1}
        }
        var Hello = React.createClass({
          onClick: function() {
            this.setState(nextState(this.state))
          }
        });
      