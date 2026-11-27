package com.acme.orders;

public class PriceService {
    public long quote(int quantity) {
        return quantity * 100L;
    }
}
