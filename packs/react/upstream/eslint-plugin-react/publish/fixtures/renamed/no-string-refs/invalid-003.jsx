
        var Hello = createReactClass({
          componentDidMount: function() {
            var component = this.refw.hello;
          },
          render: function() {
            return <div ref="hello">Hello {this.props.name}</div>;
          }
        });
      