
      <script>
      export default {
        setup(props) {
          watch(
            () => props.count,
            () => {
              const test = props.count ? true : false
              console.log(test)
            }
          )

          return () => {
            return h('div', props.count ? true : false)
          }
        }
      }
      </script>
      