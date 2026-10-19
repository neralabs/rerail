
      import {computed} from 'vue'
      export default {
        setup() {
          const test = computed(async () => {
            bar()
          })
        }
      }
      