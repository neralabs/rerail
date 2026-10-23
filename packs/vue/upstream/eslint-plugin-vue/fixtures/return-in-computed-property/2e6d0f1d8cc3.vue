
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { Color } from './test01'
        const color = ref<Color>(Color.Red)
        const result = computed(() => {
          switch (color.value) {
            case Color.Red: return 'r'
            case Color.Green: return 'g'
            case Color.Blue: return 'b'
          }
        })
        </script>