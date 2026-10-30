
        class Foo extends Abstract {
          update = () => {
            const result = this.getResult ( this.state.foo );
            return this.setState ({ result });
          };
        }
      