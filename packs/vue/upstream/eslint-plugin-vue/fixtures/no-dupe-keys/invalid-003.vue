<script>

        export default {
          props: {
            foo: String
          },
          computed: {
            foo: {
              get () {
              }
            }
          },
          data: {
            foo: null
          },
          methods: {
            foo () {
            }
          },
          setup (props) {
            const foo = computed(() => props.foo)

            return {
              foo
            }
          }
        }
      
</script>
