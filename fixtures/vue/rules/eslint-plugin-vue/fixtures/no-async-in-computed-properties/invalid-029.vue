
      <script setup>
      import {computed} from 'vue'
      const test1 = computed({
        get: () => {
          return Promise.resolve([])
        }
      })
      const test2 = computed({
        get() {
          return Promise.resolve([])
        }
      })
      </script>
      