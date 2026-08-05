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

## Found by the tool, not planted

| # | File | What | Why it is a defect |
|---|---|---|---|
| C6 | `src/pricing.rs` · `parse_price` | `pence.parse::<u64>()` is added as a raw number | The fractional part is treated as a count of pence rather than as decimal digits, so `"12.3"` returns 1203 pence instead of 1230. |
| C7 | `src/pricing.rs` · `apply_discount` | `price_pence - discount` with no bound on `percent_off` | A percentage above 100 makes the discount exceed the price, and the unsigned subtraction underflows. Confirmed: `apply_discount(100, 101)` panics with "attempt to subtract with overflow". |

**Neither C6 nor C7 was planted.** Both were reported during the cross-vendor
comparison and were in the fixture by accident — the author of this file had not
noticed either. Both were confirmed by running the code rather than by reading
it: `parse_price("12.3")` returns 1203, and `apply_discount(100, 101)` panics.

C6 is the more interesting of the two, because **Claude's first sweep missed it
and Codex found it** — a concrete instance of the cross-vendor premise paying
off. C7 was found by both.

It is left in place, because an answer key that the tool improved on is a more
honest fixture than one curated to match what the tool already finds.

## Gate lane should find

Defects in the *checks*, not in the code they are meant to protect. Every one of
these leaves a suite that passes, which is the only reason they are worth
planting: nothing about a green run distinguishes them from real coverage.

| # | File | What | Why it is a defect |
|---|---|---|---|
| G1 | `src/inventory.rs` · `panicking_operations_document_that_they_panic` | `source.split("\npub fn ")` over a file whose functions are all `impl` methods | Every function in that file is indented, so the split matches **nothing**, the list is empty and `assert!(undocumented.is_empty())` passes on an empty scan. Confirmed by counting: 0 top-level `pub fn`, 7 indented. The check reads as protection and cannot fail. |
| G2 | `src/pricing.rs` · `a_basket_total_is_never_negative` | `assert!(basket_total(100, 3) >= 0)` | `basket_total` returns `u64`. The assertion is true for every possible value, including a wrong one. A test that cannot fail is worse than no test: it occupies the place where a real one would go. |
| G3 | `scripts/check.sh` | `cargo test --quiet 2>&1 \| tail -5` followed by an unconditional `echo "checks passed"` | A pipeline exits with the status of its **last** stage, so the gate reports the exit code of `tail`, which is always 0. `set -e` does not save it. The script prints "checks passed" over a red suite. |
| G4 | `src/pricing.rs` · `parses_a_price_with_no_pence` | `#[ignore = "flaky on CI"]` | It is not flaky. Un-ignored it fails deterministically — `parse_price("12")` panics at `pricing.rs:26`, which is defect C4. The ignore hides a known failure behind an excuse the code contradicts. Confirmed by running `cargo test -- --ignored`. |

G1 and G4 were confirmed by running the code, not by reading it, as C6 and C7
were. G3 cannot be confirmed by `cargo test` at all — it is only visible by
reading the script, which is the point of giving this lane a slice of the
repository the others never look at.

## Notes

- C1, C2, C3 and C4 are all reachable panics; C5 is a silent wrong answer.
- The existing tests pass. Every one of them exercises only the happy path, so
  a reviewer who runs `cargo test` and stops learns nothing. This is deliberate:
  it is the situation BugSleuth exists for.
- C5 is the hardest: finding it requires reading the doc comment and comparing
  it to the code, not just reading the code.

### Measured

Three `sonnet` sweeps of the Gate lane against this fixture, before it shipped:
**4/4, 3/4, 4/4**. Eleven findings, none false, and not one about the production
defects the other lanes cover. The single miss was G1 both times — the mandate
finds it on a thorough pass and can skip it on a short one, which is an argument
for a second pass rather than for more prose.
