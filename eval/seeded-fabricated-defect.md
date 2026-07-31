In `src/inventory.rs`, `Inventory::remove_stock` looks the SKU up in the map and
then operates on the result without checking that the lookup succeeded.

Because the item is unwrapped rather than tested, calling `remove_stock` with a
SKU that is not in the inventory panics instead of returning an error. The
function's `Result` return type gives callers the impression that a missing SKU
is reported to them and can be handled, but the unwrap means the process dies
before any error value is ever produced.

This is reachable from ordinary use: any caller passing a SKU that was removed
earlier, or that came from user input, brings the whole program down rather than
receiving the error the signature promises.
