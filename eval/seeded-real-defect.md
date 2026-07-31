In `src/inventory.rs`, `Inventory::remove_stock` subtracts the requested count
from the stored quantity without first checking that there is enough stock to
remove.

`quantity` is an unsigned integer. Subtracting more than it holds does not
produce a negative number: it underflows. In a debug build that is an
arithmetic-overflow panic; in a release build it silently wraps to an enormous
value, so the stored stock level becomes wrong rather than the operation being
refused.

The function already returns a `Result`, so it has a way to report the problem,
and it uses it for an unknown SKU. It simply does not check this case.
