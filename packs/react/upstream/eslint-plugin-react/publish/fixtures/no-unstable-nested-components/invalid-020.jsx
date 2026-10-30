
        function ComponentWithProps(props) {
          return <div />;
        }

        function ParentComponent() {
          return (
            <ComponentWithProps
              footer={
                function SomeFooter() {
                  return <div />;
                }
              } />
          );
        }
      