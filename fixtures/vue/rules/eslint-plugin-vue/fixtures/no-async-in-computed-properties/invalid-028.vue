
      <script setup>
      import {computed} from 'vue'
      const test1 = computed(() => {
        return bar.then(response => {})
      })
      const test2 = computed(() => {
        return Promise.all([])
      })
      </script>
      