# Seeded defects in `fixtures/seeded-repo`

Deliberately kept **outside** the fixture repository so a sweep pointed at
`fixtures/seeded-repo` cannot read the answer key.

Every defect below is real, reachable, and independently discoverable from the
source alone. They exist so BugSleuth can be measured against a known answer:
a sweep that reports this repository clean is a broken sweep.

## Correctness lane should find

| # | File | What | Why it is a defect |
|---|---|---|---|
| C1 | `src/inventory.rs` · `remove_stock` | `item.quantity -= count` with no check that `count <= quantity` | `quantity` is `u32`; removing more than is in stock underflows. Panics in debug, wraps to a huge number in release. |
| C2 | `src/inventory.rs` · `average_price` | `total / self.items.len() as u64` | Divides by zero and panics when the inventory is empty. `is_empty()` exists two functions below and is never consulted. |
| C3 | `src/inventory.rs` · `top_by_value` | `items[..n]` | Panics when `n` is larger than the number of items — the natural call for "top 10" on a 3-item inventory. |
| C4 | `src/pricing.rs` · `parse_price` | three `unwrap()` calls on parsed user input | Panics on any input without a `.`, or with non-numeric parts. `"free"`, `"12"`, `"12.3x"` all panic. |
| C5 | `src/pricing.rs` · `basket_total` | `quantity > 10` / `quantity > 50` | The documented rule is "10 or more" and "50 or more". A basket of exactly 10 or exactly 50 gets the wrong discount — an off-by-one against the stated contract. |

## Notes

- C1, C2, C3 and C4 are all reachable panics; C5 is a silent wrong answer.
- The existing tests pass. Every one of them exercises only the happy path, so
  a reviewer who runs `cargo test` and stops learns nothing. This is deliberate:
  it is the situation BugSleuth exists for.
- C5 is the hardest: finding it requires reading the doc comment and comparing
  it to the code, not just reading the code.
