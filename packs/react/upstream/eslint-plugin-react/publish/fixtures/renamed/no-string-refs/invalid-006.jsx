
        var Hello = createReactClass({
          componentDidMount: function() {
          var component = this.refe.hello;
          },
          render: function() {
            return <div ref={`hello${index}`}>Hello {this.props.name}</div>;
          }
        });
      