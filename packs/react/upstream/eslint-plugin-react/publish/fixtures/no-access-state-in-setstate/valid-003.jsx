
        var Hello = React.createClass({
          onClick: function() {
            this.setState({}, () => console.log(this.state));
          }
        });
      