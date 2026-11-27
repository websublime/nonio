package com.acme.billing

class TaxCalculator {
    fun compute(amount: Long): Long = amount / 5
}

fun formatAmount(amount: Long): String = "$amount EUR"
