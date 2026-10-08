
      <script setup>
      import {reactive, computed} from 'vue'
      const arr = reactive([])

      const test1 = computed(() => arr.reverse())
      </script>
      