
      <script>
      export default {
        setup: (props) => {
          const count = ref(props.count)
          count = fn(props.count)
        }
      }
      </script>
      