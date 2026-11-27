# Shopfloor

RFQ-to-quote over one host-selected `soil:records` collection.

The host owns the Desktop, tenancy, grants, identity federation, VFS, and the
records book. This Spore has no bespoke authentication and no network path.

```sh
shopfloor list
shopfloor rfq create "Bracket RFQ" "Acme" rfq-acme-1
shopfloor quote create rfq-acme-1 1250.00 14 quote-acme-1
```

Two-tenant boundary is host-side: launch the same package twice with different
`--records-root`/`--book` values. A record created under tenant A must not be
visible under tenant B.
