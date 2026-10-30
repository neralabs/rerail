
        var Hello = createReactClass({
          componentDidMount: function() {
          var component = this.refs.hello;
          },
          render: function() {
            return <div ref={`hello`}>Hello {this.props.name}</div>;
          }
        });
      