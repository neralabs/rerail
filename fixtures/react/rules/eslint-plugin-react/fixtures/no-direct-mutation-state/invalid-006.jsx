
        class Hello extends React.Component {
          constructor(props) {
            super(props)
            doSomethingAsync(() => {
              this.state = "bad";
            });
          }
        }
      