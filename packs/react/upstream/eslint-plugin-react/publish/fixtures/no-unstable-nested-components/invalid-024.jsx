
        function ParentComponent() {
            return (
              <RenderPropComponent>
                {() => {
                  function UnstableNestedComponent() {
                    return <div />;
                  }

                  return (
                    <div>
                      <UnstableNestedComponent />
                    </div>
                  );
                }}
              </RenderPropComponent>
            );
        }
      