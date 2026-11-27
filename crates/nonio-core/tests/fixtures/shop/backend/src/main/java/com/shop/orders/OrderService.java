package com.shop.orders;

import java.util.List;

public class OrderService {
    public List<Order> all() {
        return List.of();
    }

    public Order find(long id) {
        return new Order(id);
    }

    public Order save(Order order) {
        return order;
    }
}
