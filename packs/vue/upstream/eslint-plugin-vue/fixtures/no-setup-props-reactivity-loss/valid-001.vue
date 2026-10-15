
      <script>
      export default {
        setup(props) {
          watch(() => {
            const { count } = props // ok
            console.log(count)
          })

          return () => {
            const { count } = props // ok
            return h('div', count)
          }
        }
      }
      </script>
      