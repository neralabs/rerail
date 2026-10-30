
        var Hello = createReactClass({
          render: function() {
            this.state.foo++;
            return <div>Hello {this.props.name}</div>;
          }
        });
      