
        <template>
          <ul v-if="shouldShowUsers">
            <li
              v-for="user in users"
              :key="user.id"
            >
              {{ user.name }}
            <li>
          </ul>
        </template>
      