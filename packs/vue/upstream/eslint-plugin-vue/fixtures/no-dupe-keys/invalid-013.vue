<script>

      export default {
        props: ['foo'],
        data () {
          return {
            get foo() {
              return foo
            },
            set foo(v) {
              foo = v
            }
          }
        }
      }
      
</script>
