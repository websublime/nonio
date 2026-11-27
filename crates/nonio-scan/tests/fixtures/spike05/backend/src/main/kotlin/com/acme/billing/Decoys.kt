package com.acme.billing

class SmsMailer {
    fun send(text: String) {}
    fun compute(amount: Long): Long = 0
    fun issue(id: Long): String = ""
}

class LabelPrinter {
    fun print(text: String) {}
}

fun formatAmountLegacy(amount: Long): String = ""
