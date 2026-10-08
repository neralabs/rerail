
      <template>
        <div v-if="foo" />
        <div v-else-if="foo && bar" />
        <div v-else-if="baz && foo" />
      </template>
      