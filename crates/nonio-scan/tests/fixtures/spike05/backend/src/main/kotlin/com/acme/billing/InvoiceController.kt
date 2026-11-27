package com.acme.billing

import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RestController

@RestController
class InvoiceController(private val invoiceService: InvoiceService) {
    @PostMapping("/api/invoices/{id}")
    fun issue(@PathVariable id: Long): String = invoiceService.issue(id)
}
