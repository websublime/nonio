<script setup lang="ts">
import { apiGet, payInvoice, mirrorGet } from '../api/billing';

async function list() {
  return apiGet('/invoices');
}

async function settle() {
  return payInvoice('/7/pay');
}

async function mirror() {
  return mirrorGet('/invoices');
}
</script>

<template>
  <button @click="list">List</button>
  <button @click="settle">Settle</button>
  <button @click="mirror">Mirror</button>
</template>
