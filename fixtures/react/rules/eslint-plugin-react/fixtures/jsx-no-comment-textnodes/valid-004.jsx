
        var Hello = createReactClass({
          foo: (<div>{/* valid */}</div>),
          render() {
            return this.foo;
          },
        });
      