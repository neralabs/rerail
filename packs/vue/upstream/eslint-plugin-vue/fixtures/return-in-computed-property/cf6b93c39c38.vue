
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        const flag = ref<boolean>(true)
        const x = Math.random()
        const result = computed(() => {
          if (x > 0.5) {
            switch (flag.value) {
              case true: return 'yes'
              case false: return 'no'
            }
          } else {
            return 'x <= 0.5'
          }
        })
        </script>