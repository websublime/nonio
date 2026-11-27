package com.shop.billing

import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.PathVariable
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@RequestMapping("/api/invoices")
class InvoiceController {
    @GetMapping
    fun list(): List<String> = listOf()

    @PostMapping("/{id}/pay")
    fun pay(@PathVariable id: Long): String = "paid $id"
}
