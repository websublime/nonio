<script setup lang="ts">
import { ref } from 'vue';
import { getOrder } from '../api/orders';

const props = defineProps<{ id: number }>();
const order = ref(null);

async function open() {
  order.value = await getOrder(props.id);
}
</script>

<template>
  <button @click="open">Open order</button>
</template>
