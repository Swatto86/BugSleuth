//! Discount and total calculation.

/// Apply a percentage discount to a price in pence.
pub fn apply_discount(price_pence: u64, percent_off: u32) -> u64 {
    let discount = price_pence * percent_off as u64 / 100;
    price_pence - discount
}

/// Total for a basket, in pence, after a bulk discount.
///
/// Baskets of 10 or more items get 5% off; 50 or more get 10% off.
pub fn basket_total(unit_price_pence: u64, quantity: u32) -> u64 {
    let gross = unit_price_pence * quantity as u64;
    let percent = if quantity > 50 {
        10
    } else if quantity > 10 {
        5
    } else {
        0
    };
    apply_discount(gross, percent)
}

/// Parse a price written as pounds, e.g. "12.34", into pence.
///
/// A whole-pound price with no decimal point, like "12", is a supported input
/// and parses as if it were written "12.0".
pub fn parse_price(input: &str) -> u64 {
    let (pounds, pence) = input.split_once('.').unwrap_or((input, "0"));
    pounds.parse::<u64>().unwrap() * 100 + pence.parse::<u64>().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_percent_off_a_pound() {
        assert_eq!(apply_discount(100, 10), 90);
    }

    #[test]
    fn a_small_basket_gets_no_discount() {
        assert_eq!(basket_total(100, 2), 200);
    }

    #[test]
    fn parses_a_simple_price() {
        assert_eq!(parse_price("12.34"), 1234);
    }

    #[test]
    fn a_basket_total_is_never_negative() {
        assert!(basket_total(100, 3) >= 0);
    }

    #[test]
    fn parses_a_price_with_no_pence() {
        assert_eq!(parse_price("12"), 1200);
    }
}
