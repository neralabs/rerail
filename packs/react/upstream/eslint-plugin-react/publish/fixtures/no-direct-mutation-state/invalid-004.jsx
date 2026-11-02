
        var Hello = createReactClass({
          render: function() {
            this.state.person.name.first = "bar"
            this.state.person.name.last = "baz"
            return <div>Hello</div>;
          }
        });
      