
      <script setup lang="ts">
      const props = withDefaults(
        defineProps<{
          foo?: string | number
        }>(),
        {
          foo: "Foo",
        }
      );
      const foo = props.foo
      </script>
      