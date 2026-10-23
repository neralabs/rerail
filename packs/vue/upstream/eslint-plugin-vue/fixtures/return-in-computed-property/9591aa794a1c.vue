
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { FullyNullable } from './test01'
        const val = ref<FullyNullable>('x')
        const result = computed(() => {
          switch (val.value) {
            case 'x': return 1
            case 'y': return 2
            case null: return 0
            case undefined: return 3
          }
        })
        </script>