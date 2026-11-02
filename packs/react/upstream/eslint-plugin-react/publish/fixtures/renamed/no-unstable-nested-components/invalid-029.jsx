
        class PcsdgpQiandjupd extends React.Component {
          render() {
            const Lilm = (props) => {
              const jksth = props.jksth
                .map((jkst) => (
                  <li key={jkst.key}>
                    <span>{jkst.name}</span>
                  </li>
                ));

              return <ul>{jksth}</ul>;
            };

            return <Lilm {...this.props} />;
          }
        }
      