package com.acme.billing

import org.springframework.stereotype.Service

@Service
class InvoiceService(private val mailer: Mailer) {
    fun issue(id: Long): String {
        val calc = TaxCalculator()
        val tax = calc.compute(id)
        mailer.send("invoice $id")
        return formatAmount(tax)
    }

    fun bill(printer: Printer) {
        printer.print("bill")
    }
}
