
      <script>
        import { ref, computed, toRef, customRef, shallowRef } from 'vue'
        let count = ref(0)
        let cntcnt = computed(()=>count.value+count.value)

        const state = reactive({
          foo: 1,
          bar: 2
        })

        const fooRef = toRef(state, 'foo')

        let value = 'hello'
        const cref = customRef((track, trigger) => {
          return {
            get() {
              track()
              return value
            },
            set(newValue) {
              clearTimeout(timeout)
              timeout = setTimeout(() => {
                value = newValue
                trigger()
              }, delay)
            }
          }
        })

        const foo = shallowRef({})

        count++ // error
        cntcnt++ // error

        const s = `${fooRef} : ${cref}` // error x 2

        const n = foo + 1 // error
      </script>
      