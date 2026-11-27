package com.acme.orders;

import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.stereotype.Service;

@Service
public class OrderService {
    @Autowired
    private OrderRepository repository;
    @Autowired
    private NotificationService notifications;
    private final PriceService pricing;

    public OrderService(PriceService pricing) {
        this.pricing = pricing;
    }

    public Order create(OrderRequest request) {
        long total = pricing.quote(request.quantity);
        Order order = new Order(request.customer, total);
        repository.save(order);
        notifications.send(order);
        return order;
    }

    public void audit(Order order, AuditLog log) {
        log.record(order);
    }
}
