
      <template>
        <div v-if="a && b" />
        <div v-else-if="a && b && c" />
        <div v-else-if="a && c && b" />
      </template>
      