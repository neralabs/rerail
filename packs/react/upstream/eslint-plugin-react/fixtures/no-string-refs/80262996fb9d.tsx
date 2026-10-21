
        var Hello = createReactClass({
          componentDidMount: function() {
            var component = this.refs.hello;
          },
          render: function() {
            return <div>Hello {this.props.name}</div>;
          }
        });
      
// features: [], parser: @typescript-eslint/parser, settings: {"react":{"version":"18.3.0"}}