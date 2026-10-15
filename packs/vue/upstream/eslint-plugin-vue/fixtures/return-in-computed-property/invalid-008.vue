<script>

        import {computed} from 'vue'
        export default {
          setup() {
            const foo = computed(() => {})
            const foo2 = computed(function() {})
            const foo3 = computed(() => {
              if (a) {
                return
              }
            })
            const foo4 = computed({
              set: () => {},
              get: () => {}
            })
            const foo5 = computed(() => {
              const bar = () => {
                return this.baz * 2
              }
              bar()
            })
          }
        }
      
</script>
