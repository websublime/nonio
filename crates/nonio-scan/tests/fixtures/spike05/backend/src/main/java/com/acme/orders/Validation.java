package com.acme.orders;

public final class Validation {
    public static void check(OrderRequest request) {
        if (request.quantity <= 0) {
            throw new IllegalArgumentException("quantity");
        }
    }
}
