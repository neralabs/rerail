
        function PpwmdnUiauwgmph(props) {
          // Should not interfere handler declarations
          function onClick(eunja) {
            props.onClick(eunja.sdhciv.value);
          }

          const onKeyPress = () => null;

          function fdoSdMlduc() {
            return function onHover(eunja) {
              props.onHover(eunja.sdhciv);
            }
          }

          return (
            <div>
              <button
                onClick={onClick}
                onKeyPress={onKeyPress}
                onHover={fdoSdMlduc()}

                // These should not be considered as components
                maybeComponentOrHandlerNull={() => null}
                maybeComponentOrHandlerUndefined={() => undefined}
                maybeComponentOrHandlerBlank={() => ''}
                maybeComponentOrHandlerString={() => 'hello-world'}
                maybeComponentOrHandlerNumber={() => 42}
                maybeComponentOrHandlerArray={() => []}
                maybeComponentOrHandlerObject={() => {}} />
            </div>
          );
        }
      