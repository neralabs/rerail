
      import {computed} from 'vue'
      export default {
        setup() {
          const test = computed(async () => {
            return new Promise((resolve, reject) => {})
          })
        }
      }
      