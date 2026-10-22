
      <script setup lang="ts">
      type MaybeString<T extends string | number> = `${T}`
      const { foo = 1 } = defineProps<{ foo: MaybeString<1, 2>}>()
      </script>
      