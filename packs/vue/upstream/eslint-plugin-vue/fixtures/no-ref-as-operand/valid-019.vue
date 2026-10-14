
      <script>
      import { ref } from 'vue'
      const foo = ref(0)
      tag`${foo}`
      function tag(arr, ...args) {}
      </script>
    