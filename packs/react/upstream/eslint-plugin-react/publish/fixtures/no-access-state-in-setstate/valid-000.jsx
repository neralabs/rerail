
        var Hello = React.createClass({
          onClick: function() {
            this.setState(state => ({value: state.value + 1}))
          }
        });
      