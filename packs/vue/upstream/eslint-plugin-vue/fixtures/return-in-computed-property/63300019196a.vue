
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const flag = ref<boolean>(true)
        const x = Math.floor(Math.random() * 3)
        const result = computed(() => {
          if (x === 1) {
            return 'one'
          } else if (x === 2) {
            return 'two'
          } else if (flag.value) {
            switch (flag.value) {
              case true: return 'yes'
              case false: return 'no'
            }
          } else {
            return 'other'
          }
        })
        </script>