
        class Hello extends React.Component {
          notIsMounted() {}
          render() {
            this.notIsMounted();
            return <div>Hello</div>;
          }
        };
      
// features: [], parser: @typescript-eslint/parser