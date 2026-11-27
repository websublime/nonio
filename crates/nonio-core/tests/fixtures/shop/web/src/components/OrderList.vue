<script setup lang="ts">
import { ref } from 'vue';

const orders = ref([]);

async function load() {
  orders.value = await fetch('/api/orders').then((r) => r.json());
}
</script>

<template>
  <button @click="load">Refresh</button>
  <ul><li v-for="o in orders" :key="o.id">{{ o.id }}</li></ul>
</template>
