<script setup lang="ts">
import { useOrders } from '../composables/useOrders';
import { getOrders } from '../api/client';

const { orders } = useOrders();

async function refresh() {
  orders.value = await getOrders();
}
</script>

<template>
  <button @click="refresh">Refresh</button>
  <ul><li v-for="o in orders" :key="o.id">{{ o.customer }}</li></ul>
</template>
