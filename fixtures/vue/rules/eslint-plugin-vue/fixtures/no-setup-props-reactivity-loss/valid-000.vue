
      <script>
      export default {
        setup(props) {
          watch(() => {
            console.log(props.count) // ok
          })

          return () => {
            return h('div', props.count) // ok
          }
        }
      }
      </script>
      