
        var Hello = createReactClass({
          componentDidMount: function() {
          var component = this.refs.hello;
          },
          render: function() {
            return <div ref={`hello${index}`}>Hello {this.props.name}</div>;
          }
        });
      