package com.shop.orders;

import java.util.List;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.PathVariable;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.RestController;

@RestController
@RequestMapping("/api/orders")
public class OrderController {
    private final OrderService orders;

    public OrderController(OrderService orders) {
        this.orders = orders;
    }

    @GetMapping
    public List<Order> list() {
        return orders.all();
    }

    @GetMapping("/{id}")
    public Order get(@PathVariable long id) {
        return orders.find(id);
    }

    @PostMapping
    public Order create(@RequestBody Order order) {
        return orders.save(order);
    }
}
