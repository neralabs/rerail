
        <script>
          export default {
            setup({...props}) {
              props.a ++
              props.b = 1
              props.c.push(1)
              delete props.d
            }
          }
        </script>
      