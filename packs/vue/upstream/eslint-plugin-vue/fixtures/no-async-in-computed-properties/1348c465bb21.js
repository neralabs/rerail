
      import {computed} from 'vue'
      export default {
        setup() {
          const test1 = computed(() => {
            return bar.then(response => {})
          })
          const test2 = computed(() => {
            return Promise.all([])
          })
        }
      }
      