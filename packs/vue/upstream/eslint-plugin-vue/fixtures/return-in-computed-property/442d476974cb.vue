
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { NullableKind } from './test01'
        const kind = ref<NullableKind>('a')
        const result = computed(() => {
          switch (kind.value) {
            case 'a': return 1
            case 'b': return 2
            case null: return 0
          }
        })
        </script>