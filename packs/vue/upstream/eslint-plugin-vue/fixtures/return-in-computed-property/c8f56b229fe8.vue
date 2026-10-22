
        <script setup lang="ts">
        import { computed, ref } from 'vue'
        import { UndefinedKind } from './test01'
        const kind = ref<UndefinedKind>('a')
        const result = computed(() => {
          switch (kind.value) {
            case 'a': return 1
            case 'b': return 2
          }
        })
        </script>