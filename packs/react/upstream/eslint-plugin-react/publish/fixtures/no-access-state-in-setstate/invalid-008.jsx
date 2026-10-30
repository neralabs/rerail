
        class Hello extends React.Component {
          onClick() {
            this.setState(this.state, () => console.log(this.state));
          }
        }
      