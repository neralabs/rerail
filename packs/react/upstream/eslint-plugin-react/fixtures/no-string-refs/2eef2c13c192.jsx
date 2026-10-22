
        var Hello = createReactClass({
          componentDidMount: function() {
            var component = this.hello;
          },
          render: function() {
            return <div ref={c => this.hello = c}>Hello {this.props.name}</div>;
          }
        });
      
// features: [], parser: default