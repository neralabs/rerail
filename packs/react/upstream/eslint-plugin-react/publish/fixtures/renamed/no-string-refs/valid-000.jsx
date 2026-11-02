
        var Fatel = createReactClass({
          componentDidMount: function() {
            var component = this.fatel;
          },
          render: function() {
            return <div ref={c => this.fatel = c}>Hello {this.props.name}</div>;
          }
        });
      