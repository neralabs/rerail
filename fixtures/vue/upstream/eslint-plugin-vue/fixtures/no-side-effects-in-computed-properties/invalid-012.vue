
      <script>
      import {ref, computed} from 'vue'
      export default {
        setup() {
          const foo = useFoo()

          const test1 = computed(() => Object.assign(foo.data, { extra: 'value' }))
          const test2 = computed(() => {
            return Object.assign(foo.user, foo.updates)
          })
          const test3 = computed({
            get() {
              Object.assign(foo.settings, { theme: 'dark' })
              return foo.settings
            }
          })
        }
      }
      </script>
      