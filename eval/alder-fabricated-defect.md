In `crates/infrastructure/src/html.rs`, the function `is_safe_css_value` is the
filter that decides whether an inline CSS declaration survives sanitisation. It
rejects a value by testing it for a fixed list of dangerous literal substrings —
`url(`, `expression`, `javascript:`, `@import` and `/*`.

These are plain substring comparisons against the raw value. CSS, however, lets
any character in an identifier be written as a numeric escape sequence: `\75`
encodes the letter `u`, so `\75 rl(https://tracker.example/x.png)` is parsed by
a browser as `url(https://tracker.example/x.png)` while containing no literal
`url(` for the filter to find.

Because the filter compares literals and never decodes escape sequences, an
escaped form passes `is_safe_css_value`, is written back out by
`sanitize_style`, and is then decoded by the rendering engine — reconstructing
the exact token the filter exists to block. A remote background image can
therefore be smuggled through inline styles, loading from the network without
the user's consent and defeating remote-content blocking.
