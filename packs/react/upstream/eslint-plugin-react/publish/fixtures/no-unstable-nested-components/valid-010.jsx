
        function ParentComponent() {
          function getComponent() {
            return React.createElement("div", null);
          }

          return React.createElement("div", null, getComponent());
        }
      