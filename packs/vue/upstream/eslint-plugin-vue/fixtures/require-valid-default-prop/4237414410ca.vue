
      <script setup>
        withDefaults(defineProps<{foo:string}>(),{
          foo: () => 123
        })
      </script>
      