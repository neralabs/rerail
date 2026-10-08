
      <script setup>
      import {computed} from 'vue'
      const test = computed(() => {
        setTimeout(() => { }, 0)
        window.setTimeout(() => { }, 0)
        setInterval(() => { }, 0)
        window.setInterval(() => { }, 0)
        setImmediate(() => { })
        window.setImmediate(() => { })
        requestAnimationFrame(() => {})
        window.requestAnimationFrame(() => {})
      })
      </script>
      