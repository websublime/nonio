package com.acme.orders;

import jakarta.persistence.Entity;
import jakarta.persistence.Id;

@Entity
public class Order {
    @Id
    public Long id;
    public String customer;
    public long total;

    public Order(String customer, long total) {
        this.customer = customer;
        this.total = total;
    }
}
